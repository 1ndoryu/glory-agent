-- glory-agent 0002: config clave/valor editable por el admin del producto.
-- Claves del núcleo: prompt_extra, ai_enabled_global (on|off).
-- Cada producto añade las suyas (p. ej. contacto_telefono, whatsapp_admin).
CREATE TABLE IF NOT EXISTS agent_config (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL DEFAULT '',
  updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
