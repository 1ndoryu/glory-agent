-- F10/F4: auditoría para la consola (handoff + config + import).
-- `agent_eventos` es append-only: la consola lee, nunca edita ni borra
-- (el borrado llega por CASCADE al eliminar la sesión). `actor` es staff
-- genérico (id/nombre corto), nunca PII del visitante. `detalle` JSONB
-- libre con `motivo` u otros campos según el tipo.

CREATE TABLE IF NOT EXISTS agent_eventos (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  session_id UUID NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
  tipo TEXT NOT NULL CHECK (tipo IN (
    'handoff.tomar', 'handoff.devolver', 'handoff.cerrar', 'handoff.reabrir',
    'config.cambio', 'import.puntual'
  )),
  actor TEXT NOT NULL CHECK (char_length(actor) BETWEEN 1 AND 200),
  detalle JSONB NOT NULL DEFAULT '{}',
  creado_en TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_agent_eventos_sesion
  ON agent_eventos (session_id, creado_en DESC);
