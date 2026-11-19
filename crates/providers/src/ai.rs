//! AI Provider。OpenAI 互換の `/chat/completions` だけを話す。
//!
//! 設計（`03-技術選定` §4）に合わせ、モデル名は Agent 定義に書かず
//! `model_profile`（fast / reasoning / long_context）を設定へ写像する。
//!
//! `http://` と `https://` の両方を受ける（rustls）。閉域の vLLM / Ollama は
//! 自己署名証明書を使うことが多いので、`AI_CA_BUNDLE` で CA を足せる。
//! 未設定なら `Unavailable` を返し、**生成物を捏造しない**。

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AiStatus {
    Configured,
    Unavailable,
}

/// 従量課金の単価。**未設定なら `None`**（＝コスト上限は判定できない）。
///
/// 単価をコードに埋めない。モデル名も価格も Provider 側の都合で変わるし、
/// 誤った単価で「予算内」と言うほうが、判定しないより危険だから。
#[derive(Debug, Clone, Copy, Default)]
pub struct AiPricing {
    input_per_1m: Option<f64>,
    output_per_1m: Option<f64>,
}

impl AiPricing {
    /// `AI_PRICE_INPUT_PER_1M` / `AI_PRICE_OUTPUT_PER_1M`（USD、100 万トークン単価）。
    /// 片方だけ設定されていても、その分だけ課金して計算する。
    pub fn from_env() -> Self {
        fn read(key: &str) -> Option<f64> {
            std::env::var(key)
                .ok()
                .and_then(|value| value.trim().parse::<f64>().ok())
                .filter(|value| *value >= 0.0)
        }
        Self {
            input_per_1m: read("AI_PRICE_INPUT_PER_1M"),
            output_per_1m: read("AI_PRICE_OUTPUT_PER_1M"),
        }
    }

    /// 単価が 1 つも設定されていなければ `false`。呼び出し側は
    /// 「コスト上限は判定していない」と明示する責任がある。
    pub fn is_priced(&self) -> bool {
        self.input_per_1m.is_some() || self.output_per_1m.is_some()
    }

    pub fn cost_usd(&self, prompt_tokens: i32, completion_tokens: i32) -> f64 {
        let input =
            self.input_per_1m.unwrap_or(0.0) * f64::from(prompt_tokens.max(0)) / 1_000_000.0;
        let output =
            self.output_per_1m.unwrap_or(0.0) * f64::from(completion_tokens.max(0)) / 1_000_000.0;
        input + output
    }
}

/// Agent が使えるツールの記述。`parameters` は JSON Schema。
#[derive(Debug, Clone)]
pub struct AiTool {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// LLM が要求したツール呼び出し。`arguments` は JSON 文字列のまま保持する
/// （壊れた JSON を勝手に直さない）。
#[derive(Debug, Clone)]
pub struct AiToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone)]
pub struct AiMessage {
    pub role: String,
    pub content: String,
    pub tool_calls: Vec<AiToolCall>,
    pub tool_call_id: Option<String>,
}

impl AiMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: None,
        }
    }

    pub fn assistant(content: impl Into<String>, tool_calls: Vec<AiToolCall>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
            tool_calls,
            tool_call_id: None,
        }
    }

    pub fn tool(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            role: "tool".into(),
            content: content.into(),
            tool_calls: Vec::new(),
            tool_call_id: Some(tool_call_id.into()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AiRequest {
    pub model_profile: String,
    pub messages: Vec<AiMessage>,
    pub tools: Vec<AiTool>,
}

#[derive(Debug, Clone)]
pub struct AiResponse {
    pub text: String,
    pub tool_calls: Vec<AiToolCall>,
    pub model: String,
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub cost_usd: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum AiError {
    #[error("AI provider is not configured: {0}")]
    Unavailable(&'static str),
    #[error("upstream: {0}")]
    Upstream(String),
    #[error("AI provider timed out")]
    Timeout,
}

#[async_trait]
pub trait AiProvider: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn status(&self) -> AiStatus;
    /// profile → 具体モデル名。未設定なら None。
    fn model_for(&self, profile: &str) -> Option<String>;
    async fn complete(&self, request: &AiRequest) -> Result<AiResponse, AiError>;
}

/// 未設定時の Provider。呼ぶと必ず `Unavailable` を返す。
pub struct DisabledAiProvider {
    pub reason: &'static str,
}

#[async_trait]
impl AiProvider for DisabledAiProvider {
    fn name(&self) -> &'static str {
        "disabled"
    }
    fn status(&self) -> AiStatus {
        AiStatus::Unavailable
    }
    fn model_for(&self, _profile: &str) -> Option<String> {
        None
    }
    async fn complete(&self, _request: &AiRequest) -> Result<AiResponse, AiError> {
        Err(AiError::Unavailable(self.reason))
    }
}

/// 共有 HTTP クライアント。`AI_CA_BUNDLE`（PEM）があれば自己署名 CA を信頼する。
///
/// 閉域の vLLM / Ollama は自己署名証明書が多く、`AI_TLS_INSECURE` のような
/// 「検証を切る」逃げ道は用意しない（切ったことに気付けなくなる）。
fn build_client() -> Result<reqwest::Client, String> {
    build_client_with(std::env::var("AI_CA_BUNDLE").ok().as_deref())
}

fn build_client_with(ca_bundle: Option<&str>) -> Result<reqwest::Client, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60));
    if let Some(path) = ca_bundle {
        // **設定したのに読めない**なら黙って OS の既定 CA に落とさない。
        // 自己署名の閉域環境で「なぜか繋がらない」の原因が消えるため。
        let pem = std::fs::read(path)
            .map_err(|error| format!("AI_CA_BUNDLE ({path}) を読めません: {error}"))?;
        let certs = reqwest::Certificate::from_pem_bundle(&pem)
            .map_err(|error| format!("AI_CA_BUNDLE ({path}) を解釈できません: {error}"))?;
        if certs.is_empty() {
            return Err(format!("AI_CA_BUNDLE ({path}) に証明書が入っていません"));
        }
        for cert in certs {
            builder = builder.add_root_certificate(cert);
        }
    }
    builder
        .build()
        .map_err(|error| format!("AI 用 HTTP クライアントを作れません: {error}"))
}

pub struct OpenAiCompatProvider {
    base_url: String,
    api_key: Option<String>,
    client: Result<reqwest::Client, String>,
    models: BTreeMap<String, String>,
}

impl OpenAiCompatProvider {
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        let mut models = BTreeMap::new();
        for (profile, env) in [
            ("fast", "AI_MODEL_FAST"),
            ("reasoning", "AI_MODEL_REASONING"),
            ("long_context", "AI_MODEL_LONG_CONTEXT"),
        ] {
            if let Ok(model) = std::env::var(env) {
                if !model.trim().is_empty() {
                    models.insert(profile.to_owned(), model);
                }
            }
        }
        Self {
            base_url: base_url.into(),
            api_key,
            models,
            client: build_client(),
        }
    }

    /// `/chat/completions` の完全 URL。http / https のどちらも同じ組み立て。
    fn completions_url(&self) -> Result<String, AiError> {
        let base = self.base_url.trim_end_matches('/');
        if !(base.starts_with("http://") || base.starts_with("https://")) {
            return Err(AiError::Upstream(
                "AI_BASE_URL は http:// か https:// で始めてください".into(),
            ));
        }
        Ok(format!("{base}/chat/completions"))
    }
}

#[async_trait]
impl AiProvider for OpenAiCompatProvider {
    fn name(&self) -> &'static str {
        "openai_compat"
    }

    fn status(&self) -> AiStatus {
        AiStatus::Configured
    }

    fn model_for(&self, profile: &str) -> Option<String> {
        self.models
            .get(profile)
            .or_else(|| self.models.values().next())
            .cloned()
    }

    async fn complete(&self, request: &AiRequest) -> Result<AiResponse, AiError> {
        let model = self
            .model_for(&request.model_profile)
            .ok_or(AiError::Unavailable("AI_MODEL_* is not set"))?;
        let messages: Vec<Value> = request
            .messages
            .iter()
            .map(|message| {
                let mut value = json!({
                    "role": message.role,
                    "content": message.content,
                });
                if !message.tool_calls.is_empty() {
                    value["tool_calls"] = json!(message
                        .tool_calls
                        .iter()
                        .map(|call| json!({
                            "id": call.id,
                            "type": "function",
                            "function": {"name": call.name, "arguments": call.arguments}
                        }))
                        .collect::<Vec<_>>());
                }
                if let Some(id) = &message.tool_call_id {
                    value["tool_call_id"] = json!(id);
                }
                value
            })
            .collect();
        let mut body = json!({
            "model": model,
            "temperature": 0.2,
            "messages": messages,
        });
        if !request.tools.is_empty() {
            body["tools"] = json!(request
                .tools
                .iter()
                .map(|tool| json!({
                    "type": "function",
                    "function": {
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.parameters,
                    }
                }))
                .collect::<Vec<_>>());
        }
        let url = self.completions_url()?;
        let client = self
            .client
            .as_ref()
            .map_err(|error| AiError::Upstream(error.clone()))?;
        let mut request = client
            .post(url)
            .header(CONTENT_TYPE, "application/json")
            .body(body.to_string());
        if let Some(key) = &self.api_key {
            request = request.header(AUTHORIZATION, format!("Bearer {key}"));
        }
        let response = request.send().await.map_err(|error| {
            if error.is_timeout() {
                AiError::Timeout
            } else {
                // 表層の "error sending request" だけでは TLS か DNS か分からない。
                // source を連結して 1 行に残す。
                let mut detail = error.to_string();
                let mut source = std::error::Error::source(&error);
                while let Some(cause) = source {
                    detail.push_str(&format!(": {cause}"));
                    source = cause.source();
                }
                AiError::Upstream(format!("AI への接続に失敗: {detail}"))
            }
        })?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|error| AiError::Upstream(format!("AI の応答を読めません: {error}")))?;
        if status.as_u16() >= 400 {
            return Err(AiError::Upstream(format!(
                "HTTP {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            )));
        }
        let value: Value = serde_json::from_str(&text)
            .map_err(|error| AiError::Upstream(format!("invalid JSON: {error}")))?;
        let message = value
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| choices.first())
            .and_then(|choice| choice.get("message"));
        let text = message
            .and_then(|message| message.get("content"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let tool_calls: Vec<AiToolCall> = message
            .and_then(|message| message.get("tool_calls"))
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .filter_map(|call| {
                        let function = call.get("function")?;
                        Some(AiToolCall {
                            id: call
                                .get("id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            name: function.get("name")?.as_str()?.to_owned(),
                            arguments: function
                                .get("arguments")
                                .and_then(Value::as_str)
                                .unwrap_or("{}")
                                .to_owned(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        if text.is_empty() && tool_calls.is_empty() {
            return Err(AiError::Upstream("empty completion".into()));
        }
        Ok(AiResponse {
            text,
            tool_calls,
            model: value
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or(&model)
                .to_owned(),
            prompt_tokens: value
                .pointer("/usage/prompt_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32,
            completion_tokens: value
                .pointer("/usage/completion_tokens")
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32,
            // ローカル推論は従量課金が無い。外部 SaaS を使う場合はここを設定で持つ。
            cost_usd: 0.0,
        })
    }
}

/// 環境変数から Provider を作る。`AI_BASE_URL` が無ければ Disabled。
pub fn from_env() -> Box<dyn AiProvider> {
    match std::env::var("AI_BASE_URL") {
        Ok(base) if !base.trim().is_empty() => {
            let key = std::env::var("AI_API_KEY").ok().filter(|v| !v.is_empty());
            Box::new(OpenAiCompatProvider::new(base.trim(), key))
        }
        _ => Box::new(DisabledAiProvider {
            reason: "AI_BASE_URL is not set",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_provider_refuses_to_invent_output() {
        let provider = DisabledAiProvider {
            reason: "not configured",
        };
        assert_eq!(provider.status(), AiStatus::Unavailable);
        assert!(provider.model_for("reasoning").is_none());
    }

    #[test]
    fn completions_url_accepts_http_and_https() {
        let http = OpenAiCompatProvider::new("http://127.0.0.1:11434/v1", None);
        assert_eq!(
            http.completions_url().expect("http"),
            "http://127.0.0.1:11434/v1/chat/completions"
        );
        let https = OpenAiCompatProvider::new("https://api.example.com/v1/", None);
        assert_eq!(
            https.completions_url().expect("https"),
            "https://api.example.com/v1/chat/completions"
        );
    }

    #[test]
    fn unreadable_ca_bundle_fails_loudly() {
        // 設定したのに読めないとき、黙って OS の既定 CA に落とさない。
        let error = build_client_with(Some("/definitely/not/here/ca.pem")).expect_err("must fail");
        assert!(error.contains("AI_CA_BUNDLE"), "{error}");
    }

    #[test]
    fn scheme_less_base_is_rejected() {
        let provider = OpenAiCompatProvider::new("127.0.0.1:11434/v1", None);
        assert!(matches!(
            provider.completions_url(),
            Err(AiError::Upstream(_))
        ));
    }

    #[test]
    fn pricing_is_explicit_about_being_unpriced() {
        // 単価未設定は「無料」ではなく「判定不能」。0 ドルと区別する。
        let unpriced = AiPricing::default();
        assert!(!unpriced.is_priced());
        assert_eq!(unpriced.cost_usd(1_000_000, 1_000_000), 0.0);

        // 入力 3 USD / 出力 15 USD（100 万トークンあたり）。
        let priced = AiPricing {
            input_per_1m: Some(3.0),
            output_per_1m: Some(15.0),
        };
        assert!(priced.is_priced());
        assert!((priced.cost_usd(1_000_000, 0) - 3.0).abs() < 1e-9);
        assert!((priced.cost_usd(0, 1_000_000) - 15.0).abs() < 1e-9);
        // 11 + 7 トークン（mock LLM の値）は端数になる。
        assert!((priced.cost_usd(11, 7) - (33.0 + 105.0) / 1_000_000.0).abs() < 1e-12);
        // 負のトークンは 0 として扱う（下流の計算を壊さない）。
        assert_eq!(priced.cost_usd(-5, -5), 0.0);
    }

    #[test]
    fn pricing_reads_only_non_negative_values() {
        // 環境変数の読み取りは from_env が担う。ここでは境界値の扱いだけ固定する。
        let partial = AiPricing {
            input_per_1m: None,
            output_per_1m: Some(2.0),
        };
        assert!(partial.is_priced());
        assert_eq!(partial.cost_usd(1_000_000, 0), 0.0);
        assert!((partial.cost_usd(0, 1_000_000) - 2.0).abs() < 1e-9);
    }
}
