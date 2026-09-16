/* F1: WS fanout + REST sin BD ni IA. Dos clientes en la misma sesión:
 * lo que envía A lo recibe B en vivo; REST asigna sequence y 429 al pasar el presupuesto. */

use std::time::Duration;

use futures::{SinkExt, StreamExt};
use glory_agent::{
    prompts::PromptConfig,
    providers::ProviderConfig,
    transport::{routes, AgentState},
};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMsg};
use uuid::Uuid;

async fn spawn_app() -> (String, reqwest::Client) {
    let state = AgentState::new(
        ProviderConfig {
            base_url: "http://127.0.0.1:1".to_string(),
            api_key: String::new(), // sin key: no hay llamada IA, reply None
            model: "test".to_string(),
        },
        PromptConfig::inmobiliaria_ejemplo(),
    );
    let app = routes().with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{addr}"), reqwest::Client::new())
}

fn ws_url(http_base: &str, session: Uuid) -> String {
    format!("{http_base}/agent/ws?session_id={session}").replacen("http", "ws", 1)
}

#[tokio::test]
async fn ws_fanout_two_clients_same_session() {
    let (base, _) = spawn_app().await;
    let session = Uuid::new_v4();
    let (mut a, _) = connect_async(ws_url(&base, session))
        .await
        .expect("connect A");
    let (mut b, _) = connect_async(ws_url(&base, session))
        .await
        .expect("connect B");

    a.send(WsMsg::Text(r#"{"type":"send","body":"hola"}"#.to_string()))
        .await
        .unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(5), b.next())
        .await
        .expect("timeout esperando fanout")
        .expect("stream B cerrado")
        .expect("error WS");
    let WsMsg::Text(text) = msg else {
        panic!("se esperaba Text, llegó otro frame")
    };
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(v["delivery"], "live");
    assert_eq!(v["message"]["body"], "hola");
    assert_eq!(v["message"]["sender"], "client");
    assert_eq!(v["message"]["sequence_num"], 1);
}

#[tokio::test]
async fn ws_sessions_are_isolated() {
    let (base, _) = spawn_app().await;
    let (mut a, _) = connect_async(ws_url(&base, Uuid::new_v4()))
        .await
        .expect("connect A");
    let (mut b, _) = connect_async(ws_url(&base, Uuid::new_v4()))
        .await
        .expect("connect B");

    a.send(WsMsg::Text(
        r#"{"type":"send","body":"solo-mi-sesion"}"#.to_string(),
    ))
    .await
    .unwrap();

    let res = tokio::time::timeout(Duration::from_millis(500), b.next()).await;
    assert!(res.is_err(), "B recibió mensaje de otra sesión");
}

#[tokio::test]
async fn rest_send_assigns_sequence_without_ai() {
    let (base, http) = spawn_app().await;
    let session = Uuid::new_v4();
    for expected_seq in [1, 2] {
        let resp = http
            .post(format!("{base}/agent/messages"))
            .json(&serde_json::json!({"session_id": session, "body": "hola", "visitor_key": "t1"}))
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success());
        let v: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["sequence_num"], expected_seq);
        assert!(v["reply"].is_null());
    }
}

#[tokio::test]
async fn rest_history_empty_without_pool() {
    let (base, http) = spawn_app().await;
    let resp = http
        .get(format!(
            "{base}/agent/history?session_id={}",
            Uuid::new_v4()
        ))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v, serde_json::json!([]));
}

#[tokio::test]
async fn rest_rate_limits_over_budget() {
    let (base, http) = spawn_app().await;
    let session = Uuid::new_v4();
    let mut last_status = reqwest::StatusCode::OK;
    for _ in 0..25 {
        let resp = http
            .post(format!("{base}/agent/messages"))
            .json(
                &serde_json::json!({"session_id": session, "body": "spam", "visitor_key": "t-rl"}),
            )
            .send()
            .await
            .unwrap();
        last_status = resp.status();
    }
    assert_eq!(last_status, reqwest::StatusCode::TOO_MANY_REQUESTS);
}
