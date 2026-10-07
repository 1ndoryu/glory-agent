/* Persistencia SQLx (PostgreSQL). SQL solo en los submódulos, con binds
 * (sin interpolar). Macros `query!`/`query_as!` con caché offline `.sqlx/`
 * (`SQLX_OFFLINE=true` en `.cargo/config.toml`): el build no necesita BD
 * viva; `cargo sqlx prepare` regenera el caché con BD local.
 *
 * Split por dominio (decisión 01AA-4: partir archivos, no extraer helpers):
 * `sesiones` (sesiones+mensajes+handoff+resumen_uso+inbound), `configuracion`,
 * `ciclos`, `buzon`, `eventos`. Los `pub use` preservan las rutas planas
 * (`persistence::X`) que usan `transport`, `channels/outbox` y los tests. */

pub mod buzon;
pub mod ciclos;
pub mod configuracion;
pub mod eventos;
pub mod sesiones;

pub use buzon::{enqueue_outbox, fetch_pending_outbox, mark_outbox, valid_outbox_status};
pub use ciclos::{
    ai_may_answer, get_response_cycle, should_answer_with_ai, upsert_response_cycle,
    valid_cycle_status,
};
pub use configuracion::{get_config, list_config, set_config};
pub use eventos::{listar_eventos, registrar_evento, valid_evento_tipo};
pub use sesiones::{
    create_session, delete_session, ensure_session, get_session, handoff, insert_message,
    insert_message_seq, insert_message_with_usage, list_messages, list_sessions, record_inbound,
    resumen_uso, set_session_ai, set_session_contact, set_session_status, valid_session_status,
    InboundCanal,
};

#[cfg(test)]
mod tests {
    use super::{ai_may_answer, valid_cycle_status, valid_outbox_status};

    #[test]
    fn cycle_status_rejects_unknown() {
        assert!(valid_cycle_status("waiting"));
        assert!(valid_cycle_status("answered"));
        assert!(valid_cycle_status("escalated"));
        assert!(!valid_cycle_status("open"));
        assert!(!valid_cycle_status(""));
    }

    #[test]
    fn outbox_status_rejects_unknown() {
        assert!(valid_outbox_status("pending"));
        assert!(valid_outbox_status("sent"));
        assert!(valid_outbox_status("failed"));
        assert!(!valid_outbox_status("done"));
    }

    #[test]
    fn ai_answers_unless_escalated() {
        assert!(ai_may_answer(None));
        assert!(ai_may_answer(Some("waiting")));
        assert!(ai_may_answer(Some("answered")));
        assert!(!ai_may_answer(Some("escalated")));
    }
}
