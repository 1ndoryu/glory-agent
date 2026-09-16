/* Ventana de contexto 30k: estimación barata (~4 chars/token), truncado del
 * historial antiguo y resumen opcional. El system compacto lo aporta PromptConfig. */

use serde_json::{json, Value};

/// Límite de ventana del modelo contribuidor.
pub const MAX_CONTEXT_TOKENS: usize = 30_000;

/// Estimación gruesa y rápida: 1 token ≈ 4 caracteres.
#[must_use]
pub fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4).max(1)
}

#[must_use]
pub fn estimate_messages_tokens(messages: &[Value]) -> usize {
    messages.iter().map(|m| estimate_tokens(&m.to_string())).sum()
}

/// Recorta los mensajes más antiguos hasta caber en `max_tokens`.
/// Conserva siempre el primer mensaje (system) y el último (usuario actual).
#[must_use]
pub fn truncate_to_budget(messages: Vec<Value>, max_tokens: usize) -> Vec<Value> {
    if messages.len() <= 2 || estimate_messages_tokens(&messages) <= max_tokens {
        return messages;
    }
    let mut out = Vec::with_capacity(messages.len());
    out.push(messages[0].clone());
    let mut budget = estimate_messages_tokens(&out);
    let last = messages.len() - 1;
    let mut kept_middle: Vec<Value> = Vec::new();
    for m in messages[1..last].iter().rev() {
        let cost = estimate_tokens(&m.to_string());
        if budget + cost + estimate_tokens(&messages[last].to_string()) > max_tokens {
            break;
        }
        budget += cost;
        kept_middle.push(m.clone());
    }
    kept_middle.reverse();
    out.extend(kept_middle);
    out.push(messages[last].clone());
    out
}

/// Construye el payload: system + resumen previo (si hay) + historial truncado.
#[must_use]
pub fn build_messages(system: &str, summary: Option<&str>, history: Vec<Value>) -> Vec<Value> {
    let mut messages = vec![json!({"role": "system", "content": system})];
    if let Some(s) = summary {
        if !s.trim().is_empty() {
            messages.push(json!({"role": "system", "content": format!("Resumen previo: {s}")}));
        }
    }
    messages.extend(history);
    truncate_to_budget(messages, MAX_CONTEXT_TOKENS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(i: usize) -> Value {
        json!({"role": "user", "content": format!("mensaje {i} con relleno xxxxxxxxxx")})
    }

    #[test]
    fn keeps_system_and_last_under_tight_budget() {
        let mut all = vec![json!({"role": "system", "content": "sys"})];
        for i in 0..50 {
            all.push(user_msg(i));
        }
        let out = truncate_to_budget(all, 60);
        assert_eq!(out[0]["content"], "sys");
        assert_eq!(out[out.len() - 1]["content"], "mensaje 49 con relleno xxxxxxxxxx");
        assert!(estimate_messages_tokens(&out) <= 60);
    }

    #[test]
    fn no_truncation_when_fits() {
        let all = vec![json!({"role": "system", "content": "sys"}), user_msg(1)];
        let out = truncate_to_budget(all.clone(), MAX_CONTEXT_TOKENS);
        assert_eq!(out.len(), 2);
    }
}
