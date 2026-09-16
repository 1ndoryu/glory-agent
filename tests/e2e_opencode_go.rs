/* E2E real contra OpenCode Go (Responses). Ignorado por defecto:
 * `cargo test --test e2e_opencode_go -- --ignored --nocapture`.
 * Requiere `.env` con OPENCODE_GO_API_KEY. Sin key: SKIP (no falla). */

use std::sync::{Arc, Mutex};

use axum::{extract::State, routing::post, Json, Router};
use glory_agent::providers::{call_provider, extract_first_text, ChatApiOptions, ProviderConfig};

#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<Option<String>>>>);

async fn fake_responses(
    State(seen): State<Seen>,
    headers: axum::http::HeaderMap,
    Json(_body): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    seen.0.lock().unwrap().push(
        headers
            .get("x-opencode-session")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string),
    );
    Json(serde_json::json!({"output": [
        {"type": "message", "content": [{"type": "output_text", "text": "hola-local"}]}
    ]}))
}

/// El header viaja cuando hay session_id y se omite cuando no (sin red real).
#[tokio::test]
async fn session_header_sent_only_when_present() {
    let seen = Seen::default();
    let app = Router::new()
        .route("/responses", post(fake_responses))
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let cfg = ProviderConfig {
        base_url: base,
        api_key: "test-key".to_string(),
        model: "m".to_string(),
    };
    let client = reqwest::Client::new();
    let input = vec![serde_json::json!({"role": "user", "content": "hola"})];
    let options = ChatApiOptions {
        max_output_tokens: 64,
        timeout_secs: 10,
    };
    let r1 = call_provider(&cfg, &input, None, options, Some("sesion-1"), &client)
        .await
        .unwrap();
    assert_eq!(extract_first_text(&r1).as_deref(), Some("hola-local"));
    call_provider(&cfg, &input, None, options, None, &client)
        .await
        .unwrap();
    assert_eq!(
        *seen.0.lock().unwrap(),
        vec![Some("sesion-1".to_string()), None]
    );
}

fn env_or(key: &str, fallback: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| fallback.to_string())
}

#[tokio::test]
#[ignore]
async fn e2e_responses_text_reply() {
    dotenvy::dotenv().ok();
    let api_key = std::env::var("OPENCODE_GO_API_KEY").unwrap_or_default();
    if api_key.trim().is_empty() {
        eprintln!("SKIP e2e: sin OPENCODE_GO_API_KEY");
        return;
    }
    let cfg = ProviderConfig {
        base_url: env_or("OPENCODE_GO_BASE_URL", "https://opencode.ai/zen/go/v1"),
        api_key,
        model: env_or("OPENCODE_GO_MODEL", "muse-spark-1.3-contributor"),
    };
    let client = reqwest::Client::new();
    let input =
        vec![serde_json::json!({"role": "user", "content": "Responde con exactamente: OK"})];
    let options = ChatApiOptions {
        max_output_tokens: 1024,
        timeout_secs: 120,
    };
    let resp = call_provider(
        &cfg,
        &input,
        None,
        options,
        Some("glory-agent-e2e"),
        &client,
    )
    .await
    .expect("E2E: llamada a opencode-go");
    let text = extract_first_text(&resp);
    assert!(text.is_some(), "E2E: sin texto en respuesta: {resp}");
    println!("E2E reply: {}", text.unwrap_or_default());
}
