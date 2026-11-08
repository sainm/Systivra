//! External MCP Client（stdio / HTTP）。
//!
//! 外部 Tool はローカルの固定 Capability に対応付け、名前だけでは実行しない。
//! 返却値は `untrusted=true` として隔離し、サイズ・入力・出力 Schema を検証する。

use std::collections::BTreeSet;
use std::process::Stdio;
use std::time::Duration;

use contracts::McpToolView;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue, CONTENT_TYPE};
use runtime::Capability;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

pub(crate) const DEFAULT_MCP_TIMEOUT_MS: i64 = 30_000;
pub(crate) const MAX_MCP_TIMEOUT_MS: i64 = 60_000;
pub(crate) const MAX_MCP_TOOLS: usize = 32;
pub(crate) const MAX_MCP_RESPONSE_BYTES: usize = 256 * 1024;
pub(crate) const MAX_MCP_SCHEMA_BYTES: usize = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub(crate) enum McpClientError {
    #[error("MCP config invalid: {0}")]
    InvalidConfig(String),
    #[error("MCP transport failed: {0}")]
    Transport(String),
    #[error("MCP request timed out")]
    Timeout,
    #[error("MCP response exceeds {MAX_MCP_RESPONSE_BYTES} bytes")]
    ResponseTooLarge,
    #[error("MCP protocol error: {0}")]
    Protocol(String),
    #[error("MCP tool schema invalid: {0}")]
    InvalidToolSchema(String),
    #[error("MCP input/output schema violation: {0}")]
    SchemaViolation(String),
}

impl McpClientError {
    pub(crate) fn health_status(&self) -> &'static str {
        match self {
            Self::InvalidToolSchema(_) | Self::SchemaViolation(_) => "DEGRADED",
            _ => "DOWN",
        }
    }
}

pub(crate) fn validate_config(transport: &str, config: &Value) -> Result<(), McpClientError> {
    let object = config
        .as_object()
        .ok_or_else(|| McpClientError::InvalidConfig("config は object です".into()))?;
    if let Some(timeout) = object.get("timeout_ms").and_then(Value::as_i64) {
        if !(10..=MAX_MCP_TIMEOUT_MS).contains(&timeout) {
            return Err(McpClientError::InvalidConfig(format!(
                "timeout_ms は 10〜{MAX_MCP_TIMEOUT_MS} です"
            )));
        }
    }
    match transport {
        "http" => {
            let url = object
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.trim().is_empty())
                .ok_or_else(|| McpClientError::InvalidConfig("url は必須です".into()))?;
            let parsed = reqwest::Url::parse(url)
                .map_err(|error| McpClientError::InvalidConfig(format!("url: {error}")))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(McpClientError::InvalidConfig(
                    "url は http(s) だけです".into(),
                ));
            }
            if let Some(headers) = object.get("headers") {
                parse_headers(headers)?;
            }
        }
        "stdio" => {
            object
                .get("command")
                .and_then(Value::as_str)
                .filter(|command| !command.trim().is_empty())
                .ok_or_else(|| McpClientError::InvalidConfig("command は必須です".into()))?;
            if let Some(args) = object.get("args") {
                let args = args
                    .as_array()
                    .ok_or_else(|| McpClientError::InvalidConfig("args は配列です".into()))?;
                if args.iter().any(|arg| !arg.is_string()) {
                    return Err(McpClientError::InvalidConfig(
                        "args は文字列配列です".into(),
                    ));
                }
            }
            if let Some(env) = object.get("env") {
                let env = env
                    .as_object()
                    .ok_or_else(|| McpClientError::InvalidConfig("env は object です".into()))?;
                if env.values().any(|value| !value.is_string()) {
                    return Err(McpClientError::InvalidConfig("env の値は文字列です".into()));
                }
            }
            if object.get("cwd").is_some_and(|value| !value.is_string()) {
                return Err(McpClientError::InvalidConfig("cwd は文字列です".into()));
            }
        }
        _ => {
            return Err(McpClientError::InvalidConfig(
                "transport は stdio / http です".into(),
            ));
        }
    }
    Ok(())
}

pub(crate) async fn discover(
    transport: &str,
    config: &Value,
) -> Result<Vec<McpToolView>, McpClientError> {
    validate_config(transport, config)?;
    let result = match transport {
        "http" => {
            let mut session = HttpSession::start(config).await?;
            session.request("tools/list", json!({})).await?
        }
        "stdio" => {
            let mut session = StdioSession::start(config).await?;
            let result = session.request("tools/list", json!({})).await;
            session.shutdown().await;
            result?
        }
        _ => unreachable!("transport was validated"),
    };
    normalize_tools(&result, config)
}

pub(crate) async fn call_tool(
    transport: &str,
    config: &Value,
    tool: &McpToolView,
    arguments: &Value,
) -> Result<Value, McpClientError> {
    validate_config(transport, config)?;
    validate_schema(&tool.input_schema, arguments, "arguments")?;
    let params = json!({"name": tool.name, "arguments": arguments});
    let result = match transport {
        "http" => {
            let mut session = HttpSession::start_with_timeout(config, tool.timeout_ms).await?;
            session.request("tools/call", params).await?
        }
        "stdio" => {
            let mut session = StdioSession::start_with_timeout(config, tool.timeout_ms).await?;
            let result = session.request("tools/call", params).await;
            session.shutdown().await;
            result?
        }
        _ => unreachable!("transport was validated"),
    };
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(McpClientError::Protocol(
            "tool returned isError=true".into(),
        ));
    }
    let output = if let Some(structured) = result.get("structuredContent") {
        structured.clone()
    } else {
        let text = result
            .get("content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|item| item.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        serde_json::from_str(&text).unwrap_or(Value::String(text))
    };
    validate_schema(&tool.output_schema, &output, "output")?;
    Ok(output)
}

pub(crate) fn tools_schema_hash(tools: &[McpToolView]) -> String {
    let mut canonical = tools.to_vec();
    canonical.sort_by(|left, right| left.name.cmp(&right.name));
    let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn normalize_tools(result: &Value, config: &Value) -> Result<Vec<McpToolView>, McpClientError> {
    let raw_tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| McpClientError::Protocol("tools/list result.tools がありません".into()))?;
    if raw_tools.len() > MAX_MCP_TOOLS {
        return Err(McpClientError::InvalidToolSchema(format!(
            "Tool は最大 {MAX_MCP_TOOLS} 件です"
        )));
    }
    let default_timeout = config
        .get("timeout_ms")
        .and_then(Value::as_i64)
        .unwrap_or(DEFAULT_MCP_TIMEOUT_MS);
    let mut names = BTreeSet::new();
    let mut tools = Vec::with_capacity(raw_tools.len());
    for (index, tool) in raw_tools.iter().enumerate() {
        let object = tool.as_object().ok_or_else(|| {
            McpClientError::InvalidToolSchema(format!("tools[{index}] が object ではありません"))
        })?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty() && name.len() <= 128)
            .ok_or_else(|| {
                McpClientError::InvalidToolSchema(format!("tools[{index}].name が不正です"))
            })?;
        if !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
        {
            return Err(McpClientError::InvalidToolSchema(format!(
                "tools[{index}].name に使用できない文字があります"
            )));
        }
        if !names.insert(name.to_owned()) {
            return Err(McpClientError::InvalidToolSchema(format!(
                "Tool 名が重複しています: {name}"
            )));
        }
        let capability_name = tool_capability(tool).ok_or_else(|| {
            McpClientError::InvalidToolSchema(format!("tools[{index}].capability がありません"))
        })?;
        let capability = Capability::parse(capability_name).ok_or_else(|| {
            McpClientError::InvalidToolSchema(format!("未知の capability です: {capability_name}"))
        })?;
        let input_schema = tool_schema(tool, "inputSchema", "input_schema").ok_or_else(|| {
            McpClientError::InvalidToolSchema(format!("tools[{index}].inputSchema がありません"))
        })?;
        validate_schema_shape(&input_schema, &format!("tools[{index}].inputSchema"))?;
        let output_schema =
            tool_schema(tool, "outputSchema", "output_schema").unwrap_or_else(|| json!({}));
        validate_schema_shape(&output_schema, &format!("tools[{index}].outputSchema"))?;
        let side_effects = tool_side_effects(tool)?;
        let requested_approval = tool
            .get("approval_required")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let approval_required =
            requested_approval || matches!(side_effects.as_str(), "write" | "destructive");
        let timeout_ms = tool
            .get("timeout_ms")
            .and_then(Value::as_i64)
            .unwrap_or(default_timeout);
        if !(10..=MAX_MCP_TIMEOUT_MS).contains(&timeout_ms) {
            return Err(McpClientError::InvalidToolSchema(format!(
                "tools[{index}].timeout_ms は 10〜{MAX_MCP_TIMEOUT_MS} です"
            )));
        }
        tools.push(McpToolView {
            name: name.to_owned(),
            capability: capability.as_str().to_owned(),
            description: tool
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            input_schema,
            output_schema,
            timeout_ms,
            side_effects,
            approval_required,
            server: None,
        });
    }
    Ok(tools)
}

fn tool_capability(tool: &Value) -> Option<&str> {
    tool.get("capability")
        .and_then(Value::as_str)
        .or_else(|| tool.pointer("/_meta/capability").and_then(Value::as_str))
}

fn tool_schema(tool: &Value, camel: &str, snake: &str) -> Option<Value> {
    let schema = tool.get(camel).or_else(|| tool.get(snake))?;
    schema.is_object().then(|| schema.clone())
}

fn tool_side_effects(tool: &Value) -> Result<String, McpClientError> {
    let side_effects = tool
        .get("side_effects")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            let annotations = tool.get("annotations")?;
            if annotations.get("destructiveHint").and_then(Value::as_bool) == Some(true) {
                Some("destructive".to_owned())
            } else if annotations.get("readOnlyHint").and_then(Value::as_bool) == Some(true) {
                Some("read".to_owned())
            } else {
                Some("write".to_owned())
            }
        })
        .unwrap_or_else(|| "write".to_owned());
    if !matches!(
        side_effects.as_str(),
        "none" | "read" | "write" | "destructive"
    ) {
        return Err(McpClientError::InvalidToolSchema(
            "side_effects が不正です".into(),
        ));
    }
    Ok(side_effects)
}

fn validate_schema_shape(schema: &Value, path: &str) -> Result<(), McpClientError> {
    let object = schema
        .as_object()
        .ok_or_else(|| McpClientError::InvalidToolSchema(format!("{path} は object です")))?;
    if serde_json::to_vec(schema).map_or(true, |bytes| bytes.len() > MAX_MCP_SCHEMA_BYTES) {
        return Err(McpClientError::InvalidToolSchema(format!(
            "{path} は {MAX_MCP_SCHEMA_BYTES} bytes 以下です"
        )));
    }
    if let Some(value_type) = object.get("type") {
        if !matches!(
            value_type.as_str(),
            Some("object" | "array" | "string" | "number" | "integer" | "boolean" | "null")
        ) {
            return Err(McpClientError::InvalidToolSchema(format!(
                "{path}.type が不正です"
            )));
        }
    }
    Ok(())
}

fn validate_schema(schema: &Value, value: &Value, path: &str) -> Result<(), McpClientError> {
    if schema.as_object().is_some_and(Map::is_empty) {
        return Ok(());
    }
    if schema.get("$ref").is_some() {
        return Err(McpClientError::SchemaViolation(format!(
            "{path}: $ref は外部 Client では解決できません"
        )));
    }
    if let Some(expected) = schema.get("type").and_then(Value::as_str) {
        let matches = match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "number" => value.is_number(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            _ => false,
        };
        if !matches {
            return Err(McpClientError::SchemaViolation(format!(
                "{path}: {expected} を期待しました"
            )));
        }
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array) {
        if !values.contains(value) {
            return Err(McpClientError::SchemaViolation(format!(
                "{path}: enum に一致しません"
            )));
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema.get("required").and_then(Value::as_array) {
            for field in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(field) {
                    return Err(McpClientError::SchemaViolation(format!(
                        "{path}.{field} がありません"
                    )));
                }
            }
        }
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (key, child) in object {
            if let Some(child_schema) = properties.get(key) {
                validate_schema(child_schema, child, &format!("{path}.{key}"))?;
            } else if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
                return Err(McpClientError::SchemaViolation(format!(
                    "{path}.{key} は許可されていません"
                )));
            }
        }
    }
    if let Some(array) = value.as_array() {
        if let Some(items) = schema.get("items") {
            for (index, value) in array.iter().enumerate() {
                validate_schema(items, value, &format!("{path}[{index}]"))?;
            }
        }
    }
    Ok(())
}

fn parse_headers(value: &Value) -> Result<HeaderMap, McpClientError> {
    let object = value
        .as_object()
        .ok_or_else(|| McpClientError::InvalidConfig("headers は object です".into()))?;
    let mut headers = HeaderMap::new();
    for (name, value) in object {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| McpClientError::InvalidConfig(format!("header {name}: {error}")))?;
        let value = value.as_str().ok_or_else(|| {
            McpClientError::InvalidConfig(format!("header {name} の値は文字列です"))
        })?;
        let value = HeaderValue::from_str(value)
            .map_err(|error| McpClientError::InvalidConfig(format!("header {name}: {error}")))?;
        headers.insert(name, value);
    }
    Ok(headers)
}

fn config_timeout(config: &Value, override_ms: Option<i64>) -> Duration {
    Duration::from_millis(
        override_ms
            .or_else(|| config.get("timeout_ms").and_then(Value::as_i64))
            .unwrap_or(DEFAULT_MCP_TIMEOUT_MS)
            .clamp(10, MAX_MCP_TIMEOUT_MS) as u64,
    )
}

struct HttpSession {
    client: reqwest::Client,
    url: String,
    headers: HeaderMap,
    session_id: Option<String>,
    next_id: i64,
}

impl HttpSession {
    async fn start(config: &Value) -> Result<Self, McpClientError> {
        Self::start_with_timeout(config, DEFAULT_MCP_TIMEOUT_MS).await
    }

    async fn start_with_timeout(config: &Value, timeout_ms: i64) -> Result<Self, McpClientError> {
        let timeout = config_timeout(config, Some(timeout_ms));
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|error| McpClientError::Transport(error.to_string()))?;
        let url = config
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| McpClientError::InvalidConfig("url は必須です".into()))?
            .to_owned();
        let headers = config
            .get("headers")
            .map(parse_headers)
            .transpose()?
            .unwrap_or_default();
        let mut session = Self {
            client,
            url,
            headers,
            session_id: None,
            next_id: 1,
        };
        session.request("initialize", initialize_params()).await?;
        session
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(session)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        let response = self.post(&request, true).await?;
        rpc_result(response, method)
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), McpClientError> {
        let request = json!({"jsonrpc":"2.0","method":method,"params":params});
        self.post(&request, false).await.map(|_| ())
    }

    async fn post(&mut self, request: &Value, expect_body: bool) -> Result<Value, McpClientError> {
        let mut headers = self.headers.clone();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
        if let Some(session_id) = &self.session_id {
            headers.insert(
                "mcp-session-id",
                HeaderValue::from_str(session_id).map_err(|error| {
                    McpClientError::Protocol(format!("invalid session id: {error}"))
                })?,
            );
        }
        let response = self
            .client
            .post(&self.url)
            .headers(headers)
            .json(request)
            .send()
            .await
            .map_err(map_reqwest_error)?;
        if !response.status().is_success() {
            return Err(McpClientError::Transport(format!(
                "HTTP {}",
                response.status()
            )));
        }
        if let Some(session_id) = response.headers().get("mcp-session-id") {
            self.session_id = session_id.to_str().ok().map(str::to_owned);
        }
        if !expect_body {
            return Ok(Value::Null);
        }
        if response.content_length().unwrap_or_default() as usize > MAX_MCP_RESPONSE_BYTES {
            return Err(McpClientError::ResponseTooLarge);
        }
        let bytes = response.bytes().await.map_err(map_reqwest_error)?;
        if bytes.len() > MAX_MCP_RESPONSE_BYTES {
            return Err(McpClientError::ResponseTooLarge);
        }
        serde_json::from_slice(&bytes)
            .map_err(|error| McpClientError::Protocol(format!("JSON: {error}")))
    }
}

struct StdioSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: i64,
    timeout: Duration,
}

impl StdioSession {
    async fn start(config: &Value) -> Result<Self, McpClientError> {
        Self::start_with_timeout(config, DEFAULT_MCP_TIMEOUT_MS).await
    }

    async fn start_with_timeout(config: &Value, timeout_ms: i64) -> Result<Self, McpClientError> {
        let command = config
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| McpClientError::InvalidConfig("command は必須です".into()))?;
        let mut process = Command::new(command);
        if let Some(args) = config.get("args").and_then(Value::as_array) {
            process.args(args.iter().filter_map(Value::as_str));
        }
        if let Some(cwd) = config.get("cwd").and_then(Value::as_str) {
            process.current_dir(cwd);
        }
        if let Some(env) = config.get("env").and_then(Value::as_object) {
            for (name, value) in env {
                if let Some(value) = value.as_str() {
                    process.env(name, value);
                }
            }
        }
        let mut child = process
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|error| McpClientError::Transport(format!("spawn: {error}")))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpClientError::Transport("stdin を開けません".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpClientError::Transport("stdout を開けません".into()))?;
        let mut session = Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            timeout: config_timeout(config, Some(timeout_ms)),
        };
        session.request("initialize", initialize_params()).await?;
        session
            .notify("notifications/initialized", json!({}))
            .await?;
        Ok(session)
    }

    async fn request(&mut self, method: &str, params: Value) -> Result<Value, McpClientError> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        self.write_line(&request).await?;
        let mut line = String::new();
        let read = tokio::time::timeout(self.timeout, self.stdout.read_line(&mut line))
            .await
            .map_err(|_| McpClientError::Timeout)?
            .map_err(|error| McpClientError::Transport(error.to_string()))?;
        if read == 0 {
            return Err(McpClientError::Transport("stdio が終了しました".into()));
        }
        if line.len() > MAX_MCP_RESPONSE_BYTES {
            return Err(McpClientError::ResponseTooLarge);
        }
        let response: Value = serde_json::from_str(&line)
            .map_err(|error| McpClientError::Protocol(format!("JSON: {error}")))?;
        rpc_result(response, method)
    }

    async fn notify(&mut self, method: &str, params: Value) -> Result<(), McpClientError> {
        self.write_line(&json!({"jsonrpc":"2.0","method":method,"params":params}))
            .await
    }

    async fn write_line(&mut self, value: &Value) -> Result<(), McpClientError> {
        let mut bytes = serde_json::to_vec(value)
            .map_err(|error| McpClientError::Protocol(error.to_string()))?;
        bytes.push(b'\n');
        tokio::time::timeout(self.timeout, async {
            self.stdin.write_all(&bytes).await?;
            self.stdin.flush().await
        })
        .await
        .map_err(|_| McpClientError::Timeout)?
        .map_err(|error| McpClientError::Transport(error.to_string()))
    }

    async fn shutdown(&mut self) {
        let _ = self.child.kill().await;
        let _ = self.child.wait().await;
    }
}

fn initialize_params() -> Value {
    json!({
        "protocolVersion": "2024-11-05",
        "capabilities": {},
        "clientInfo": {"name": "systivra", "version": "0.1.0"}
    })
}

fn rpc_result(response: Value, method: &str) -> Result<Value, McpClientError> {
    if let Some(error) = response.get("error") {
        return Err(McpClientError::Protocol(format!(
            "{method}: {}",
            truncate(&error.to_string(), 300)
        )));
    }
    response
        .get("result")
        .cloned()
        .ok_or_else(|| McpClientError::Protocol(format!("{method}: result がありません")))
}

fn map_reqwest_error(error: reqwest::Error) -> McpClientError {
    if error.is_timeout() {
        McpClientError::Timeout
    } else {
        McpClientError::Transport(error.to_string())
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    text.chars().take(max).collect::<String>() + "..."
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(input: Value, output: Value) -> McpToolView {
        McpToolView {
            name: "external_project_list".into(),
            capability: "project.list".into(),
            description: "x".into(),
            input_schema: input,
            output_schema: output,
            timeout_ms: 1000,
            side_effects: "read".into(),
            approval_required: false,
            server: None,
        }
    }

    #[test]
    fn schema_hash_changes_with_input_or_output_schema() {
        let base = tool(json!({"type":"object"}), json!({}));
        let changed = tool(
            json!({"type":"object","required":["mode"]}),
            json!({"type":"object"}),
        );
        assert_ne!(tools_schema_hash(&[base]), tools_schema_hash(&[changed]));
    }

    #[test]
    fn validates_required_and_additional_properties() {
        let schema = json!({
            "type": "object",
            "required": ["project_id"],
            "properties": {"project_id": {"type": "string"}},
            "additionalProperties": false
        });
        assert!(validate_schema(&schema, &json!({"project_id":"p"}), "input").is_ok());
        assert!(validate_schema(&schema, &json!({}), "input").is_err());
        assert!(validate_schema(
            &schema,
            &json!({"project_id":"p","project_id_from_model":"x"}),
            "input"
        )
        .is_err());
    }

    #[test]
    fn rejects_ref_schemas_instead_of_silently_accepting() {
        assert!(validate_schema(&json!({"$ref":"schema://x"}), &json!({}), "output").is_err());
    }

    #[test]
    fn normalizes_external_tool_and_derives_approval() {
        let result = json!({
            "tools": [{
                "name": "external_tool",
                "capability": "test.run_execute",
                "description": "dangerous",
                "inputSchema": {
                    "type": "object",
                    "required": ["project_id"],
                    "properties": {"project_id": {"type": "string"}}
                },
                "outputSchema": {"type": "object"},
                "side_effects": "destructive"
            }]
        });
        let tools = normalize_tools(&result, &json!({})).unwrap();
        assert_eq!(tools[0].capability, "test.run_execute");
        assert!(tools[0].approval_required);
        assert_eq!(tools[0].side_effects, "destructive");
    }

    #[test]
    fn rejects_unknown_external_capability() {
        let result = json!({
            "tools": [{
                "name": "sql_query",
                "capability": "sql.query",
                "inputSchema": {"type": "object"}
            }]
        });
        assert!(normalize_tools(&result, &json!({})).is_err());
    }
}
