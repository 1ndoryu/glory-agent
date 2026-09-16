/* Hub de sesiones: fanout in-memory por session_id (single-instance v1).
 * Difiere del hub genérico de glory-rs (por user_id): aquí la unidad es la
 * conversación, con sequence_num monotónico para ordenar y detectar huecos. */

use std::sync::{
    atomic::{AtomicI64, Ordering},
    Arc,
};

use axum::extract::ws::Message;
use dashmap::DashMap;
use tokio::sync::mpsc::UnboundedSender;
use uuid::Uuid;

use crate::models::WsServerMessage;

#[derive(Debug, Clone)]
struct SessionEntry {
    tx: UnboundedSender<Message>,
}

#[derive(Clone, Default)]
pub struct ChatHub {
    sessions: Arc<DashMap<Uuid, Vec<SessionEntry>>>,
    sequences: Arc<DashMap<Uuid, AtomicI64>>,
}

impl ChatHub {
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(DashMap::new()),
            sequences: Arc::new(DashMap::new()),
        }
    }

    /// Siguiente sequence_num monotónico de la sesión (empieza en 1).
    pub fn next_sequence(&self, session_id: Uuid) -> i64 {
        let entry = self.sequences.entry(session_id).or_insert_with(|| AtomicI64::new(1));
        entry.fetch_add(1, Ordering::SeqCst)
    }

    pub fn subscribe(&self, session_id: Uuid, tx: UnboundedSender<Message>) {
        self.sessions.entry(session_id).or_default().push(SessionEntry { tx });
    }

    /// Retorna conexiones entregadas; purga las cerradas.
    pub fn broadcast(&self, session_id: Uuid, msg: &WsServerMessage) -> usize {
        let Ok(json) = serde_json::to_string(msg) else {
            return 0;
        };
        let mut delivered = 0usize;
        let mut remove_session = false;
        if let Some(mut bucket) = self.sessions.get_mut(&session_id) {
            bucket.retain(|entry| {
                if entry.tx.send(Message::Text(json.clone())).is_ok() {
                    delivered += 1;
                    true
                } else {
                    false
                }
            });
            remove_session = bucket.is_empty();
        }
        if remove_session {
            self.sessions.remove(&session_id);
        }
        delivered
    }

    #[must_use]
    pub fn connection_count(&self, session_id: Uuid) -> usize {
        self.sessions.get(&session_id).map_or(0, |bucket| bucket.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ChatMessage, SenderType};
    use chrono::Utc;
    use tokio::sync::mpsc::unbounded_channel;

    fn sample_msg(seq: i64) -> ChatMessage {
        ChatMessage {
            id: Uuid::new_v4(),
            session_id: Uuid::new_v4(),
            sender: SenderType::Ai.as_str().to_string(),
            body: "hola".to_string(),
            sequence_num: seq,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn sequence_is_monotonic_per_session() {
        let hub = ChatHub::new();
        let session = Uuid::new_v4();
        assert_eq!(hub.next_sequence(session), 1);
        assert_eq!(hub.next_sequence(session), 2);
        assert_eq!(hub.next_sequence(Uuid::new_v4()), 1);
    }

    #[test]
    fn broadcast_delivers_and_prunes_dead() {
        let hub = ChatHub::new();
        let session = Uuid::new_v4();
        let (tx_live, mut rx_live) = unbounded_channel::<Message>();
        let (tx_dead, rx_dead) = unbounded_channel::<Message>();
        drop(rx_dead);
        hub.subscribe(session, tx_live);
        hub.subscribe(session, tx_dead);
        let delivered = hub.broadcast(session, &WsServerMessage::live(sample_msg(1)));
        assert_eq!(delivered, 1);
        assert_eq!(hub.connection_count(session), 1);
        assert!(rx_live.try_recv().is_ok());
    }
}
