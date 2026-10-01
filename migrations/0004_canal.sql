-- F10/F2: columnas de canal en mensajes + idempotencia en outbox.
-- `canal` agnóstico (`web`, `whatsapp`, ...); PII solo como hash HMAC
-- (`numero_destino_hash`, secreto en el bin/adapter, nunca aquí).
-- Dedup de ingress por (canal, id_externo) cuando el canal da id.
-- `imported` (F11): filas de import puntual, siempre sin usage.

ALTER TABLE agent_messages
  ADD COLUMN IF NOT EXISTS canal TEXT NOT NULL DEFAULT 'web',
  ADD COLUMN IF NOT EXISTS numero_destino_hash TEXT,
  ADD COLUMN IF NOT EXISTS id_externo TEXT,
  ADD COLUMN IF NOT EXISTS via TEXT,
  ADD COLUMN IF NOT EXISTS media_ref JSONB,
  ADD COLUMN IF NOT EXISTS imported BOOLEAN NOT NULL DEFAULT FALSE;

-- El import puntual no trae usage (fuera de la ventana de 30k del plan §6b).
ALTER TABLE agent_messages
  ADD CONSTRAINT chk_agent_messages_import_sin_usage
  CHECK (imported = FALSE OR (input_tokens IS NULL AND output_tokens IS NULL));

-- Dedup parcial: solo filas con id externo del canal (NULL = sin dedup).
CREATE UNIQUE INDEX IF NOT EXISTS uq_agent_messages_canal_externo
  ON agent_messages (canal, id_externo) WHERE id_externo IS NOT NULL;

-- Outbox: clave de idempotencia por intento de envío (la calcula el
-- adapter/bin con HMAC; el núcleo solo la exige única). Filas viejas NULL.
ALTER TABLE agent_outbox
  ADD COLUMN IF NOT EXISTS idempotency_key TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS uq_agent_outbox_idem
  ON agent_outbox (idempotency_key) WHERE idempotency_key IS NOT NULL;
