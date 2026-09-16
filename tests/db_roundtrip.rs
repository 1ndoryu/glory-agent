/* Roundtrip vivo contra Postgres (F4-live). Ignorado por defecto:
 * requiere DATABASE_URL (ver .env; instancia local puerto 5433).
 * Ejecutar: cargo test --test db_roundtrip -- --ignored --nocapture */

use glory_agent::persistence::{
    create_session, delete_session, enqueue_outbox, fetch_pending_outbox, get_response_cycle,
    insert_message, list_messages, mark_outbox, should_answer_with_ai, upsert_response_cycle,
};

async fn pool() -> sqlx::PgPool {
    dotenvy::dotenv().ok();
    let url = std::env::var("DATABASE_URL").expect("F4-live: falta DATABASE_URL en .env");
    sqlx::PgPool::connect(&url)
        .await
        .expect("F4-live: no conecta a Postgres")
}

#[tokio::test]
#[ignore]
async fn db_roundtrip_session_messages_cycle_outbox() {
    let pool = pool().await;

    let session = create_session(&pool, Some("Visitante Test"), Some("test@example.com"))
        .await
        .expect("create_session");
    assert!(session.ai_enabled);

    let m1 = insert_message(&pool, session.id, "client", "Hola, busco piso", 1)
        .await
        .expect("insert m1");
    let m2 = insert_message(&pool, session.id, "ai", "Hola, ¿qué zona?", 2)
        .await
        .expect("insert m2");
    assert_eq!(m1.sequence_num, 1);
    assert_eq!(m2.sequence_num, 2);

    let history = list_messages(&pool, session.id, 50).await.expect("history");
    assert_eq!(history.len(), 2);

    // Secuencia duplicada viola UNIQUE(session_id, sequence_num).
    assert!(insert_message(&pool, session.id, "client", "dup", 2)
        .await
        .is_err());

    // Toma humana: por defecto la IA responde; escalado la frena.
    assert!(should_answer_with_ai(&pool, session.id).await.expect("ai?"));
    upsert_response_cycle(&pool, session.id, "escalated")
        .await
        .expect("escalate");
    let cycle = get_response_cycle(&pool, session.id)
        .await
        .expect("get cycle")
        .expect("cycle existe");
    assert_eq!(cycle.status, "escalated");
    assert!(!should_answer_with_ai(&pool, session.id).await.expect("ai?"));

    // Outbox: enqueue → pendiente → sent.
    let entry = enqueue_outbox(&pool, "lead", serde_json::json!({"contact": "test"}))
        .await
        .expect("enqueue");
    assert_eq!(entry.status, "pending");
    let pending = fetch_pending_outbox(&pool, 10).await.expect("pending");
    assert!(pending.iter().any(|e| e.id == entry.id));
    let sent = mark_outbox(&pool, entry.id, "sent")
        .await
        .expect("mark sent");
    assert_eq!(sent.status, "sent");

    // Limpieza: la sesión de prueba no contamina (CASCADE cubre mensajes+ciclo).
    let gone = delete_session(&pool, session.id).await.expect("cleanup");
    assert_eq!(gone, 1);
    println!("F4-live roundtrip: OK");
}
