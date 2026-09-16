-- glory-agent v1: sesiones, mensajes, outbox, ciclos de respuesta.
-- Tablas con prefijo agent_ para convivir con el producto.

CREATE TABLE IF NOT EXISTS agent_sessions (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  visitor_name TEXT,
  contact TEXT,
  status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open','escalated','closed')),
  ai_enabled BOOLEAN NOT NULL DEFAULT TRUE,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS agent_messages (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  session_id UUID NOT NULL REFERENCES agent_sessions(id) ON DELETE CASCADE,
  sender TEXT NOT NULL CHECK (sender IN ('client','ai','staff','system')),
  body TEXT NOT NULL CHECK (char_length(body) BETWEEN 1 AND 8000),
  sequence_num BIGINT NOT NULL,
  created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
  UNIQUE (session_id, sequence_num)
);
CREATE INDEX IF NOT EXISTS idx_agent_messages_session ON agent_messages (session_id, sequence_num DESC);

CREATE TABLE IF NOT EXISTS agent_response_cycles (
  session_id UUID PRIMARY KEY REFERENCES agent_sessions(id) ON DELETE CASCADE,
  status TEXT NOT NULL DEFAULT 'waiting' CHECK (status IN ('waiting','answered','escalated')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS agent_outbox (
  id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
  kind TEXT NOT NULL,
  payload JSONB NOT NULL DEFAULT '{}',
  status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','sent','failed')),
  created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_agent_outbox_status ON agent_outbox (status, created_at);
