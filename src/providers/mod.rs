/* Providers OpenAI-compat. Primero opencode-go (muse-spark-1.3-contribuidor).
 * Patrón portado de NAKOMI/src/services/ai_providers.rs (344l): body builder +
 * un proveedor con retry simple. Sin acoplar DeepSeek/Groq/Gemini en v1. */

use serde_json::Value;

#[derive(Clone, Copy)]
pub struct ChatApiOptions {
    pub temperature: f32,
    pub max_tokens: u32,
    pub top_p: f32,
    pub timeout_secs: u64,
}

impl ChatApiOptions {
    #[must_use]
    pub const fn standard() -> Self {
        Self {
            temperature: 0.7,
            max_tokens: 800,
            top_p: 0.9,
            timeout_secs: 30,
        }
    }

    #[must_use]
    pub const fn terse(max_tokens: u32) -> Self {
        Self {
            temperature: 0.2,
            max_tokens,
            top_p: 0.8,
            timeout_secs: 15,
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
            base_url: "https://api.opencode-go.example/v1".to_string(),
            api_key,
            model: "muse-spark-1.3-contribuidor".to_string(),
        }
    }

    #[must_use]
    pub fn completions_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            api_key: String::new(),
            model: "muse-spark-1.3-contribuidor".to_string(),
        }
    }
}

#[must_use]
pub fn build_chat_body(
    model: &str,
    messages: &[Value],
    tools: Option<&Value>,
    options: ChatApiOptions,
) -> Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": messages,
        "temperature": options.temperature,
        "max_tokens": options.max_tokens,
        "top_p": options.top_p,
        "stream": false,
    });
    if let Some(t) = tools {
        body["tools"] = t.clone();
    }
    body
}

/// Extrae el texto del primer choice OpenAI-compat. `None` si vacío/inválido.
#[must_use]
pub fn extract_first_text(resp: &Value) -> Option<String> {
    let text = resp
        .get("choices")?
        .as_array()?
        .first()?
        .get("message")?
        .get("content")?
        .as_str()?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

pub async fn call_provider(
    config: &ProviderConfig,
    messages: &[Value],
    tools: Option<&Value>,
    options: ChatApiOptions,
    client: &reqwest::Client,
) -> Result<Value, String> {
    if config.api_key.trim().is_empty() {
        return Err("AI: falta api_key (OPENCODE_GO_API_KEY)".to_string());
    }
    let body = build_chat_body(&config.model, messages, tools, options);
    let key_hint_len = config.api_key.len().min(8);
    let key_hint = &config.api_key[..key_hint_len];
    let resp = client
        .post(config.completions_url())
        .timeout(std::time::Duration::from_secs(options.timeout_secs))
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
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
        tracing::info!("AI OK: modelo={}, key={key_hint}...", config.model);
        return Ok(json);
    }
    let text = resp.text().await.unwrap_or_default();
    tracing::error!("AI HTTP {status} key {key_hint}...: {text}");
    Err(format!("AI HTTP {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_uses_model_and_limits() {
        let messages = vec![serde_json::json!({"role": "user", "content": "hola"})];
        let body = build_chat_body(
            "muse-spark-1.3-contribuidor",
            &messages,
            None,
            ChatApiOptions::standard(),
        );
        assert_eq!(body["model"], "muse-spark-1.3-contribuidor");
        assert_eq!(body["max_tokens"], 800);
        assert_eq!(body["stream"], false);
    }

    #[test]
    fn extract_text_rejects_empty() {
        let empty = serde_json::json!({"choices": [{"message": {"content": "  "}}]});
        assert!(extract_first_text(&empty).is_none());
        let ok = serde_json::json!({"choices": [{"message": {"content": "hola"}}]});
        assert_eq!(extract_first_text(&ok).as_deref(), Some("hola"));
    }

    #[test]
    fn completions_url_trims_slash() {
        let cfg = ProviderConfig {
            base_url: "https://x.test/v1/".to_string(),
            api_key: "k".to_string(),
            model: "m".to_string(),
        };
        assert_eq!(cfg.completions_url(), "https://x.test/v1/chat/completions");
    }
}
