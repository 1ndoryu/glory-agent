/* Provider OpenCode Go para Muse Spark, vía Responses API.
 * Evidencia (Agente/documentacion/muse-opencode-go-vscode-bucle-2026-09-03.md):
 * - `POST /v1/chat/completions` con muse-spark → HTTP 500. Solo `/v1/responses` (200).
 * - Sin `reasoningSummary` (400) ni `previous_response_id` (400); stateless.
 * - `include: ["reasoning.encrypted_content"]` aceptado.
 * Auth `Bearer <key>`. La key NUNCA se loguea (ni prefijos). */

use serde_json::Value;

#[derive(Clone, Copy)]
pub struct ChatApiOptions {
    pub max_output_tokens: u32,
    pub timeout_secs: u64,
}

impl ChatApiOptions {
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            max_output_tokens: 800,
            timeout_secs: 60,
        }
    }

    #[must_use]
    pub const fn terse(max_output_tokens: u32) -> Self {
        Self {
            max_output_tokens,
            timeout_secs: 60,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
}

impl ProviderConfig {
    #[must_use]
    pub fn opencode_go(api_key: String) -> Self {
        Self {
            base_url: "https://opencode.ai/zen/go/v1".to_string(),
            api_key,
            model: "muse-spark-1.3-contributor".to_string(),
        }
    }

    #[must_use]
    pub fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url.trim_end_matches('/'))
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self::opencode_go(String::new())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: String,
    pub call_id: Option<String>,
}

/* [F0] Usage del turno (Responses `usage`): suma por llamada cuando el
 * loop de tools hace varias. Ausente en respuestas viejas/fakes → None. */
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TurnUsage {
    pub input_tokens: i32,
    pub output_tokens: i32,
}

impl TurnUsage {
    #[must_use]
    pub fn saturating_add(self, other: Self) -> Self {
        Self {
            input_tokens: self.input_tokens.saturating_add(other.input_tokens),
            output_tokens: self.output_tokens.saturating_add(other.output_tokens),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResponsesOutput {
    pub text: Option<String>,
    pub function_calls: Vec<FunctionCall>,
    pub usage: Option<TurnUsage>,
}

/// Lee `resp.usage.{input_tokens,output_tokens}` (u64 del provider → i32
/// saturado para la columna). `None` si falta o no es número.
#[must_use]
pub fn parse_responses_usage(resp: &Value) -> Option<TurnUsage> {
    let usage = resp.get("usage")?;
    let to_i32 = |v: &Value| -> Option<i32> {
        v.as_u64()
            .map(|n| i32::try_from(n).unwrap_or(i32::MAX))
            .or_else(|| v.as_i64().map(|n| i32::try_from(n).unwrap_or(0).max(0)))
    };
    Some(TurnUsage {
        input_tokens: to_i32(usage.get("input_tokens")?)?,
        output_tokens: to_i32(usage.get("output_tokens")?)?,
    })
}

/// Convierte tools estilo chat (`{type:function, function:{...}}`) al formato
/// Responses (`{type:function, name, description, parameters}`). Lo desconocido
/// pasa tal cual.
#[must_use]
pub fn to_responses_tools(tools: &Value) -> Value {
    let Some(arr) = tools.as_array() else {
        return tools.clone();
    };
    Value::Array(
        arr.iter()
            .map(|t| {
                if let Some(f) = t.get("function") {
                    let mut out = serde_json::json!({"type": "function"});
                    for key in ["name", "description", "parameters"] {
                        if let Some(v) = f.get(key) {
                            out[key] = v.clone();
                        }
                    }
                    out
                } else {
                    t.clone()
                }
            })
            .collect(),
    )
}

#[must_use]
pub fn build_responses_body(
    model: &str,
    input: &[Value],
    tools: Option<&Value>,
    options: ChatApiOptions,
) -> Value {
    let mut body = serde_json::json!({
        "model": model,
        "input": input,
        "store": false,
        "max_output_tokens": options.max_output_tokens,
        "include": ["reasoning.encrypted_content"],
    });
    if let Some(t) = tools {
        body["tools"] = to_responses_tools(t);
    }
    body
}

/// Parsea `output[]`: primer `output_text` + todos los `function_call` +
/// `usage` del turno (si viene).
#[must_use]
pub fn parse_responses_output(resp: &Value) -> ResponsesOutput {
    let mut out = ResponsesOutput {
        usage: parse_responses_usage(resp),
        ..Default::default()
    };
    let Some(items) = resp.get("output").and_then(Value::as_array) else {
        return out;
    };
    for item in items {
        match item.get("type").and_then(Value::as_str) {
            Some("message") if out.text.is_none() => {
                if let Some(content) = item.get("content").and_then(Value::as_array) {
                    for part in content {
                        if part.get("type").and_then(Value::as_str) == Some("output_text") {
                            if let Some(text) = part.get("text").and_then(Value::as_str) {
                                let trimmed = text.trim();
                                if !trimmed.is_empty() {
                                    out.text = Some(trimmed.to_string());
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            Some("function_call") => {
                if let Some(name) = item.get("name").and_then(Value::as_str) {
                    out.function_calls.push(FunctionCall {
                        name: name.to_string(),
                        arguments: item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or("{}")
                            .to_string(),
                        call_id: item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    });
                }
            }
            _ => {}
        }
    }
    out
}

/// Extrae el texto de la respuesta Responses. `None` si vacío/inválido.
#[must_use]
pub fn extract_first_text(resp: &Value) -> Option<String> {
    parse_responses_output(resp).text
}

pub async fn call_provider(
    config: &ProviderConfig,
    messages: &[Value],
    tools: Option<&Value>,
    options: ChatApiOptions,
    session_id: Option<&str>,
    client: &reqwest::Client,
) -> Result<Value, String> {
    if config.api_key.trim().is_empty() {
        return Err("AI: falta api_key (OPENCODE_GO_API_KEY)".to_string());
    }
    let body = build_responses_body(&config.model, messages, tools, options);
    // x-opencode-session: afinidad por conversación (un chat = un valor).
    // Sin este header el relay Go responde 400 MissingSessionID.
    let mut req = client
        .post(config.responses_url())
        .timeout(std::time::Duration::from_secs(options.timeout_secs))
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json");
    if let Some(sid) = session_id {
        req = req.header("x-opencode-session", sid);
    }
    let resp = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("AI error de red: {e}"))?;
    let status = resp.status();
    if status.is_success() {
        let json: Value = resp
            .json()
            .await
            .map_err(|e| format!("AI parse error: {e}"))?;
        tracing::info!("AI OK: modelo={}", config.model);
        return Ok(json);
    }
    let text = resp.text().await.unwrap_or_default();
    let snippet: String = text.chars().take(300).collect();
    tracing::error!("AI HTTP {status}: {snippet}");
    Err(format!("AI HTTP {status}: {snippet}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input() -> Vec<Value> {
        vec![serde_json::json!({"role": "user", "content": "hola"})]
    }

    #[test]
    fn body_uses_model_and_limits() {
        let body = build_responses_body(
            "muse-spark-1.3-contributor",
            &sample_input(),
            None,
            ChatApiOptions::standard(),
        );
        assert_eq!(body["model"], "muse-spark-1.3-contributor");
        assert_eq!(body["max_output_tokens"], 800);
        assert_eq!(body["store"], false);
        assert!(body.get("temperature").is_none());
        assert!(body.get("messages").is_none());
    }

    #[test]
    fn tools_map_to_responses_shape() {
        let chat_tools = serde_json::json!([
            {"type": "function", "function": {"name": "crear_lead", "description": "d", "parameters": {"type": "object"}}}
        ]);
        let body = build_responses_body(
            "m",
            &sample_input(),
            Some(&chat_tools),
            ChatApiOptions::terse(64),
        );
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "crear_lead");
        assert!(body["tools"][0].get("function").is_none());
    }

    #[test]
    fn extract_text_from_responses_output() {
        let ok = serde_json::json!({"output": [
            {"type": "message", "content": [{"type": "output_text", "text": "  hola  "}]}
        ]});
        assert_eq!(extract_first_text(&ok).as_deref(), Some("hola"));
        let empty = serde_json::json!({"output": [
            {"type": "message", "content": [{"type": "output_text", "text": "   "}]}
        ]});
        assert!(extract_first_text(&empty).is_none());
        assert!(extract_first_text(&serde_json::json!({})).is_none());
    }

    #[test]
    fn parse_collects_function_calls() {
        let resp = serde_json::json!({"output": [
            {"type": "function_call", "name": "crear_lead", "arguments": "{\"a\":1}", "call_id": "c1"}
        ]});
        let out = parse_responses_output(&resp);
        assert!(out.text.is_none());
        assert_eq!(out.function_calls.len(), 1);
        assert_eq!(out.function_calls[0].name, "crear_lead");
        assert_eq!(out.function_calls[0].call_id.as_deref(), Some("c1"));
    }

    #[test]
    fn usage_se_lee_y_falta_da_none() {
        let con =
            serde_json::json!({"usage": {"input_tokens": 120, "output_tokens": 35}, "output": []});
        assert_eq!(
            parse_responses_output(&con).usage,
            Some(TurnUsage {
                input_tokens: 120,
                output_tokens: 35
            })
        );
        assert!(parse_responses_output(&serde_json::json!({"output": []}))
            .usage
            .is_none());
        assert!(
            parse_responses_output(&serde_json::json!({"usage": {"input_tokens": 1}}))
                .usage
                .is_none()
        );
    }

    #[test]
    fn usage_suma_satura_sin_desbordar() {
        let a = TurnUsage {
            input_tokens: i32::MAX,
            output_tokens: 10,
        };
        let b = TurnUsage {
            input_tokens: 5,
            output_tokens: i32::MAX,
        };
        let s = a.saturating_add(b);
        assert_eq!(s.input_tokens, i32::MAX);
        assert_eq!(s.output_tokens, i32::MAX);
    }

    #[test]
    fn responses_url_trims_slash() {
        let cfg = ProviderConfig {
            base_url: "https://x.test/v1/".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
        };
        assert_eq!(cfg.responses_url(), "https://x.test/v1/responses");
    }
}
