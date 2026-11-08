//! AI Provider。OpenAI 互換の `/chat/completions` だけを話す。
//!
//! 設計（`03-技術選定` §4）に合わせ、モデル名は Agent 定義に書かず
//! `model_profile`（fast / reasoning / long_context）を設定へ写像する。
//!
//! TLS は扱わない（`http://` のみ）。閉域の vLLM / Ollama を想定し、
//! 外部 SaaS を使う場合は手前に TLS 終端を置く。未設定なら
//! `Unavailable` を返し、**生成物を捏造しない**。

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

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

pub struct OpenAiCompatProvider {
    base_url: String,
    api_key: Option<String>,
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
        }
    }

    fn endpoint(&self) -> Result<(String, u16, String), AiError> {
        let rest = self
            .base_url
            .strip_prefix("http://")
            .ok_or_else(|| AiError::Upstream("only http:// base URLs are supported".into()))?;
        let (authority, base_path) = match rest.find('/') {
            Some(index) => (&rest[..index], &rest[index..]),
            None => (rest, ""),
        };
        if authority.is_empty() {
            return Err(AiError::Upstream("empty AI base URL".into()));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.contains(']') => (
                host.to_owned(),
                port.parse::<u16>()
                    .map_err(|_| AiError::Upstream("invalid AI base port".into()))?,
            ),
            _ => (authority.to_owned(), 80),
        };
        let path = format!("{}/chat/completions", base_path.trim_end_matches('/'));
        Ok((host, port, path))
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
        let (host, port, path) = self.endpoint()?;
        let body = body.to_string();
        let raw = http_post(&host, port, &path, body.as_bytes(), self.api_key.as_deref()).await?;
        let value: Value = serde_json::from_slice(&raw)
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

async fn http_post(
    host: &str,
    port: u16,
    path: &str,
    body: &[u8],
    api_key: Option<&str>,
) -> Result<Vec<u8>, AiError> {
    let mut stream =
        tokio::time::timeout(Duration::from_secs(10), TcpStream::connect((host, port)))
            .await
            .map_err(|_| AiError::Timeout)?
            .map_err(|error| AiError::Upstream(format!("connect {host}:{port}: {error}")))?;

    let auth = api_key
        .map(|key| format!("Authorization: Bearer {key}\r\n"))
        .unwrap_or_default();
    let head = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n{auth}\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .await
        .map_err(|error| AiError::Upstream(error.to_string()))?;
    stream
        .write_all(body)
        .await
        .map_err(|error| AiError::Upstream(error.to_string()))?;

    let mut buffer = Vec::new();
    tokio::time::timeout(Duration::from_secs(60), stream.read_to_end(&mut buffer))
        .await
        .map_err(|_| AiError::Timeout)?
        .map_err(|error| AiError::Upstream(error.to_string()))?;

    let split = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| AiError::Upstream("malformed HTTP response".into()))?;
    let head_text = String::from_utf8_lossy(&buffer[..split]).to_ascii_lowercase();
    let status = head_text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .unwrap_or(0);
    let mut payload = buffer[split + 4..].to_vec();
    if head_text.contains("transfer-encoding: chunked") {
        payload = decode_chunked(&payload)?;
    }
    if status >= 400 {
        return Err(AiError::Upstream(format!(
            "HTTP {status}: {}",
            String::from_utf8_lossy(&payload)
                .chars()
                .take(200)
                .collect::<String>()
        )));
    }
    Ok(payload)
}

/// chunked は文字境界を保証しないのでバイト列で連結する。
fn decode_chunked(raw: &[u8]) -> Result<Vec<u8>, AiError> {
    let mut out = Vec::new();
    let mut rest = raw;
    loop {
        let newline = rest
            .windows(2)
            .position(|window| window == b"\r\n")
            .ok_or_else(|| AiError::Upstream("bad chunk header".into()))?;
        let header = String::from_utf8_lossy(&rest[..newline]);
        let size = usize::from_str_radix(header.trim().split(';').next().unwrap_or("0"), 16)
            .map_err(|_| AiError::Upstream("bad chunk size".into()))?;
        if size == 0 {
            break;
        }
        let start = newline + 2;
        if rest.len() < start + size {
            return Err(AiError::Upstream("truncated chunk".into()));
        }
        out.extend_from_slice(&rest[start..start + size]);
        rest = &rest[(start + size).min(rest.len())..];
        if rest.starts_with(b"\r\n") {
            rest = &rest[2..];
        }
    }
    Ok(out)
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
    fn endpoint_parses_http_base() {
        let provider = OpenAiCompatProvider::new("http://127.0.0.1:11434/v1", None);
        let (host, port, path) = provider.endpoint().expect("endpoint");
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 11434);
        assert_eq!(path, "/v1/chat/completions");
    }

    #[test]
    fn https_is_rejected_explicitly() {
        let provider = OpenAiCompatProvider::new("https://api.example.com/v1", None);
        assert!(matches!(provider.endpoint(), Err(AiError::Upstream(_))));
    }

    #[test]
    fn chunked_body_is_decoded() {
        let raw = b"4\r\n{\"a\"\r\n3\r\n:1}\r\n0\r\n\r\n";
        let out = decode_chunked(raw).expect("chunked");
        assert_eq!(String::from_utf8_lossy(&out), "{\"a\":1}");
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
