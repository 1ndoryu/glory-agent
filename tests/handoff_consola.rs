/* F4 vivo contra Postgres (ignorado por defecto): handoff atómico con
 * auditoría, eventos append-only, `resumen_uso` y `list_config`.
 * Requiere DATABASE_URL (ver .env; instancia local puerto 5433).
 * Ejecutar (con lease del guard): etapa viva temporal del gate. */

use glory_agent::persistence::{
    create_session, delete_session, handoff, insert_message_with_usage, list_config,
    listar_eventos, registrar_evento, resumen_uso, set_config,
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
async fn handoff_tomar_devolver_con_auditoria() {
    let pool = pool().await;
    let sesion = create_session(&pool, None, None).await.expect("crear");
    assert_eq!(sesion.status, "open");

    let tomada = handoff(
        &pool,
        sesion.id,
        "tomar",
        "ana",
        Some("cliente pide humano"),
    )
    .await
    .expect("tomar");
    assert_eq!(tomada.status, "escalated");

    let eventos = listar_eventos(&pool, sesion.id, 10).await.expect("listar");
    assert_eq!(eventos.len(), 1);
    assert_eq!(eventos[0].tipo, "handoff.tomar");
    assert_eq!(eventos[0].actor, "ana");
    assert_eq!(eventos[0].detalle["motivo"], "cliente pide humano");

    let devuelta = handoff(&pool, sesion.id, "devolver", "ana", None)
        .await
        .expect("devolver");
    assert_eq!(devuelta.status, "open");

    let err = handoff(&pool, sesion.id, "devolver", "ana", None).await;
    assert!(err.is_err(), "devolver en open debe fallar");

    let cerrada = handoff(&pool, sesion.id, "cerrar", "ana", None)
        .await
        .expect("cerrar");
    assert_eq!(cerrada.status, "closed");
    let abierta = handoff(&pool, sesion.id, "reabrir", "ana", None)
        .await
        .expect("reabrir");
    assert_eq!(abierta.status, "open");

    delete_session(&pool, sesion.id).await.expect("cleanup");
}

#[tokio::test]
#[ignore]
async fn eventos_uso_y_config_para_consola() {
    let pool = pool().await;
    let sesion = create_session(&pool, None, None).await.expect("crear");

    registrar_evento(
        &pool,
        sesion.id,
        "import.puntual",
        "ana",
        &serde_json::json!({ "mensajes": 12 }),
    )
    .await
    .expect("evento");
    let err = registrar_evento(
        &pool,
        sesion.id,
        "spam.borrar",
        "ana",
        &serde_json::json!({}),
    )
    .await;
    assert!(err.is_err(), "tipo fuera de allowlist debe fallar");

    insert_message_with_usage(&pool, sesion.id, "client", "hola", 1, None, None)
        .await
        .expect("client");
    insert_message_with_usage(&pool, sesion.id, "ai", "buenas", 2, Some(120), Some(35))
        .await
        .expect("ai");
    let resumen = resumen_uso(&pool, sesion.id, None, None)
        .await
        .expect("uso");
    assert_eq!(resumen.mensajes_ai, 1, "solo cuenta mensajes ai");
    assert_eq!(resumen.input_tokens, 120);
    assert_eq!(resumen.output_tokens, 35);

    let tag = uuid::Uuid::new_v4().to_string();
    let clave = format!("consola_prueba_{tag}");
    set_config(&pool, &clave, "1").await.expect("set");
    let todas = list_config(&pool).await.expect("list");
    assert!(todas.iter().any(|c| c.key == clave));

    delete_session(&pool, sesion.id).await.expect("cleanup");
}
