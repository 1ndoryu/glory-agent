/* F2 vivo contra Postgres (ignorado por defecto): idempotencia del outbox,
 * drenado por el worker y dedup de inbound por (canal, id_externo).
 * Requiere DATABASE_URL (ver .env; instancia local puerto 5433).
 * Ejecutar (con lease del guard): cargo test --test canal_outbox -- --ignored --nocapture */

use std::future::Future;
use std::pin::Pin;

use glory_agent::channels::outbox::{drain_outbox_once, enqueue_send};
use glory_agent::channels::{Outbound, Receipt, Sender, SenderError};
use glory_agent::persistence::{
    create_session, delete_session, enqueue_outbox, fetch_pending_outbox, record_inbound,
    InboundCanal,
};

async fn pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("F4-live: falta DATABASE_URL en .env");
    sqlx::PgPool::connect(&url)
        .await
        .expect("F4-live: no conecta a Postgres")
}

struct TestSender {
    enviados: std::sync::Mutex<Vec<Outbound>>,
}

impl Sender for TestSender {
    fn enviar<'a>(
        &'a self,
        out: &'a Outbound,
    ) -> Pin<Box<dyn Future<Output = Result<Receipt, SenderError>> + Send + 'a>> {
        Box::pin(async move {
            out.validar()?;
            self.enviados.lock().expect("lock").push(out.clone());
            Ok(Receipt { message_id: None })
        })
    }
}

#[tokio::test]
#[ignore]
async fn outbox_idempotente_y_drena() {
    let pool = pool().await;
    let tag = uuid::Uuid::new_v4().to_string();
    let sender = TestSender {
        enviados: std::sync::Mutex::new(vec![]),
    };
    let out = Outbound {
        destino: "34111111111".to_string(),
        texto: "hola".to_string(),
        media_url: None,
        via: "wa_b".to_string(),
        idempotency_key: format!("test-{tag}"),
    };

    let e1 = enqueue_send(&pool, &out).await.expect("enqueue 1");
    let e2 = enqueue_send(&pool, &out).await.expect("enqueue 2");
    assert_eq!(e1.id, e2.id, "misma clave = misma fila");

    let stats = drain_outbox_once(&pool, &sender, 50).await.expect("drain");
    assert_eq!(stats.sent, 1);
    assert_eq!(sender.enviados.lock().expect("lock").len(), 1);

    let pendientes = fetch_pending_outbox(&pool, 100).await.expect("pendientes");
    assert!(!pendientes.iter().any(|e| e.id == e1.id));
}

#[tokio::test]
#[ignore]
async fn inbound_dedup_por_canal_id_externo() {
    let pool = pool().await;
    let session = create_session(&pool, None, None).await.expect("sesion");
    let tag = uuid::Uuid::new_v4().to_string();
    let id_ext = format!("baileys-{tag}");
    let meta = InboundCanal {
        canal: "whatsapp".to_string(),
        numero_destino_hash: Some("hash-destino".to_string()),
        id_externo: Some(id_ext),
        via: Some("wa_b".to_string()),
        ..InboundCanal::default()
    };

    let primero = record_inbound(&pool, session.id, "client", "hola", 1, &meta)
        .await
        .expect("inbound 1");
    assert!(primero.is_some());

    let duplicado = record_inbound(&pool, session.id, "client", "hola", 2, &meta)
        .await
        .expect("inbound 2");
    assert!(duplicado.is_none(), "duplicado (canal, id) se descarta");

    let otro_canal = InboundCanal {
        canal: "web".to_string(),
        numero_destino_hash: None,
        ..meta.clone()
    };
    let web = record_inbound(&pool, session.id, "client", "hola", 3, &otro_canal)
        .await
        .expect("inbound 3");
    assert!(web.is_some(), "misma id en otro canal no es duplicado");

    delete_session(&pool, session.id).await.expect("limpia");
}

#[tokio::test]
#[ignore]
async fn drain_omite_kind_desconocido() {
    let pool = pool().await;
    let sender = TestSender {
        enviados: std::sync::Mutex::new(vec![]),
    };
    let otro = enqueue_outbox(&pool, "otro-worker", serde_json::json!({"a": 1}))
        .await
        .expect("otro kind");
    let stats = drain_outbox_once(&pool, &sender, 50).await.expect("drain");
    assert!(stats.skipped >= 1);
    let pendientes = fetch_pending_outbox(&pool, 100).await.expect("pendientes");
    assert!(pendientes.iter().any(|e| e.id == otro.id));
}
