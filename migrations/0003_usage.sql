-- glory-agent F0 (pista 279A-2): usage exacto por turno del provider.
-- La API Responses devuelve `usage: {input_tokens, output_tokens}` por
-- llamada; se guarda en el mensaje `ai` del turno para que cada producto
-- (p. ej. `uso_mensajes`) deje de estimar. Nullable: filas viejas y
-- mensajes no-IA quedan en NULL y cada producto decide el fallback.
ALTER TABLE agent_messages
  ADD COLUMN IF NOT EXISTS input_tokens INTEGER CHECK (input_tokens IS NULL OR input_tokens >= 0),
  ADD COLUMN IF NOT EXISTS output_tokens INTEGER CHECK (output_tokens IS NULL OR output_tokens >= 0);
