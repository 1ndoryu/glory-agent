# Plan plataforma agnóstica chat + IA → glory-agent (solo diseño)

Fecha: 2026-10-01 · Estado: activo · Ubicación: `area-trabajo/glory-agent/`
Decisión usuaria 2026-10-01: glory-agent como herramienta agnóstica que
facilite todo lo de chat con agentes IA para cualquier app. Solo diseño,
sin código. Ampliación jam aprobada.
Consumidor actual: `MN-Inmobiliaria/`. Núcleo destino: `glory-agent/`.

## 1. Objetivo

`glory-agent` ofrece la infraestructura hecha para que cualquier app:
atienda chat web + WhatsApp con IA, envíe mensajes (texto/foto/audio),
gestione sesiones por cliente×canal, pase a humano y retome, mida uso
y opere desde consola. La app solo pone: credenciales, prompt, tools de
negocio y sus tablas propias. Sin mover código en este plan.

## 2. No-goals (este plan)

- No mover ni editar `.rs` ni `.mjs`.
- No portar prompts, tools ni tablas de negocio a `glory-agent`.
- No meter Baileys dentro del crate Rust (va como adaptador aparte).
- No migrar Nakomi.

## 3. Estado verificado

- `glory-agent` hoy es chat web + IA texto: `transport::routes/AgentState`,
  `providers` Responses API `/responses`, `tools::ToolExecutor/ToolCtx`,
  `persistence` directo a `agent_*`, `prompts::PromptConfig`.
  Cero WhatsApp: ni webhook, ni media, ni STT, ni `wa_a/wa_b`.
- Todo WhatsApp vive en `MN-Inmobiliaria`:
  entrada `gateway/src/sesion.mjs:118` → `backend.mjs:4` →
  `src/handlers/whatsapp.rs:618 webhook`, `reparto()`, `repartir_y_vincular()`;
  media `whatsapp.rs:174 cuerpo_media`, `252 descargar_y_guardar`,
  `352 describir_y_anexar`, `412 transcribir_y_anexar` con `ia.rs:559/659`;
  turno en background tras 2xx + acuse 6s `programar_acuse()` +
  `partir_respuesta()`; salida `services/alerta_whatsapp.rs:10 vigilar`
  poll 15s `agent_outbox kind=whatsapp` → POST `/send`.
- Negocio incrustado en el mismo flujo: `chat.rs:88 prompt_config()`,
  `chat_tools.rs:24 definiciones()` 9 tools, tarjetas, `cliente.rs`,
  `chat_staff.rs`, consola dueña.

## 4. Lo que la app recibe hecho (plataforma)

- Un solo turno agentico: recibir → pensar con tools → responder, igual
  en web y WhatsApp. Reusa `responder_turno_persistido` + `ToolExecutor`.
- Canales por adaptador: `Channel { web | whatsapp | ... }`, sesión =
  cliente×canal, historial real, `MediaRef` de primera clase
  (hoy puente `[foto]/[audio]` como prefijo texto).
- Envío unificado `Outbound { destino, texto, media_url?, via }` + outbox
  con reintentos, backoff y estados `pending/sent/failed`.
- Media hecha: guardar/servir URL firmada, transcribir voz, describir foto.
- Handoff hecho: `activa → consultando → delegada`, triple freno,
  toma/devolución staff, sin perder hilo.
- Operación hecha: config (`prompt_extra`, kill-switch, topes, teléfonos),
  uso exacto (`input/output_tokens`), auditoría, endpoints admin para consola.

## 5. Lo que la app sigue poniendo (negocio, nunca al núcleo)

- Prompt, identidad, tono, idioma, reglas (ej. texto-plano WhatsApp MN).
- Tools de negocio + sus tablas (ej. 9 tools inmuebles, `clientes`,
  `canal_sesiones/atencion/uso`, fichas, visitas).
- Números, modos (`completo/inicial`), mutes, `whatsapp_admin`, QR y deploy.

Mezclas actuales a separar (no copiar tal cual):
- `whatsapp.rs` junta reparto + storage + STT + acuse + outbox.
- `chat_tools.rs` junta executor genérico + SQL inmuebles + enqueue + espejo.
- `alerta_whatsapp.rs` junta worker genérico + ficha comercial MN.

## 6. Diseño (núcleo + adaptadores, sin BD de negocio dentro)

```rust
struct MediaRef { id, mime, size, url_firmada_ttl, session_id } // sin bytes: evita OOM con tope 10 MiB × N sesiones
struct NormalizedInbound { canal, numero_destino, remitente, texto, nombre?, media?: MediaRef, tipo }
struct Outbound { destino, texto, media_url?, via: String, idempotency_key } // via libre + allowlist por adapter (wa_a/wa_b solo ejemplo MN)
trait Ingress {
  async fn normalizar(raw: InRaw) -> Result<NormalizedInbound, IngressError>; // timeout 20s, tope 10 MiB, mime allowlist
}
trait Sender {
  async fn enviar(out: Outbound) -> Result<Receipt, SenderError>; // timeout 20s, reintento con backoff
}
trait MediaStore { async fn guardar(bytes, mime, session_id) -> Result<MediaRef, MediaError>; }
trait Transcriber { async fn audio_a_texto(media: &MediaRef) -> Result<String, SttError>; }
trait Describer { async fn foto_a_texto(media: &MediaRef) -> Result<String, VisionError>; }
trait Resolver { async fn sesion_por_canal(cliente_id, canal, numero) -> Result<Uuid, ResolverError>; }
```

- Firmas reales: todo `Result` con `timeout`, `tamaño máx` y `mime allowlist`;
  `session_id` viaja en `MediaRef` para trazabilidad (reto supervisor-thinker).
- SLO wpp como contrato (no riesgo suelto): 2xx rápido + turno en
  background, acuse 6s, `partir_respuesta()`, anti-eco TTL 180s MAX 500,
  QR por sesión, presupuesto STT/media cobrado en núcleo por sesión/día,
  hub in-memory = single-instance explícito.
- Frontera PII: plano solo en memoria del adapter; a BD solo
  `numero_destino_hash` con HMAC (teléfono enumerable, SHA puro no basta),
  secreto HMAC en `bin`/adapter, nunca en `lib`.
- Núcleo: conversación, sesión, historial, tools, providers, prompts,
  outbox, handoff, config, uso. Nunca `clientes` ni `UPLOAD_DIR` ni secreto
  `X-Gateway-Secret` ni webhook WhatsApp dentro del crate.
- Adaptadores oficiales: `wpp` (Baileys/Evolution), `media` local/S3,
  `stt` Whisper, `sender` HTTP con `via`. La app elige e inyecta.
- Base actual: `MN-Inmobiliaria/gateway/src/` (`config.mjs:21`,
  `sesion.mjs:118/131`, `eco.mjs:17`, `backend.mjs:4`, `media.mjs`) se
  convierte en adaptador `wpp` config-driven: N sesiones, `respondeIA` por
  sesión, anti-eco TTL 180s MAX 500, `POST /send`, QR por sesión.
- Anti-YAGNI: un solo consumidor real (MN); el 2º es ficticio y solo valida
  que nada inmobiliario quede en el núcleo (prompt/tools/tablas fuera).

## 6b. Persistencia (no romper F5/F9)

- Mapa: `session_id` web existente convive con `cliente×canal×numero`;
  `Resolver` reutiliza `ensure_session` idempotente (`ca53feb`) y FK a
  `agent_sessions`, sin duplicar hilos.
- Migración `agent_*`: `0004_canal.sql` exacta con `canal`,
  `numero_destino_hash`, `UNIQUE (canal, baileys_msg_id)`, `via: String`
  + allowlist por adapter (`wa_a/wa_b` solo ejemplo MN), `media_ref`,
  `imported BOOL`; `clientes/canal_sesiones/atencion` quedan en MN.
- Regla import: `imported=true` → `usage NULL` y fuera de ventana 30k
  salvo pedido explícito; dedup por `(canal, baileys_msg_id)`.
- Outbox: `pending → sent/failed` + `idempotency_key` + backoff +
  `mark_outbox` en boundary; `HISTORIAL_TURNOS=30` y `usage` exacto intactos.

## 7. Fases futuras (cuando se autorice código)

- F1: tipos + traits + `MediaRef` + test con fakes.
- F2: outbox genérico + worker + `Sender` inyectable + reintentos.
- F3: adaptador `wpp` config-driven + media + STT hooks.
- F4: handoff + config/uso/auditoría como API lista para consola.
- F5: MN migra como primer consumidor delgado (detalle sin romper abajo).

## 7b. F5 sin romper MN (strangler, solo diseño)

- Principio: MN sigue funcionando igual durante toda la migración; cada
  paso es reversible con un flag y sin borrar código viejo hasta el final.
- Paso 0 foto: congelar conducta actual (webhook:618, reparto A/B,
  acuse 6s, `partir_respuesta`, `vigilar` 15s, `HISTORIAL_TURNOS=30`,
  `usage` exacto) como tests de caracterización en MN, sin tocar `glory-agent`.
- Paso 1 sombra: comparar solo `NormalizedInbound`+sesión+distribución
  `usage`, nunca texto exacto (LLM no determinista); `ToolExecutor` en
  stub read-only, STT/visión cacheados, coste sombra fuera de presupuesto;
  flag `GLORY_SHADOW=1`; si difiere, se registra y se sigue con lo viejo.
- Paso 2 outbox única fuente: escribe solo al nuevo con
  `idempotency_key=HMAC(canal|baileys_msg_id|destino|hash texto)` UNIQUE +
  chequeo antes de `POST /send` (TTL 7d); el viejo lee, nunca doble envío;
  dry-run del sender nuevo primero.
- Paso 3 corte sticky por sesión: `wa_b/inicial` primero, luego
  `wa_a/completo`; parar, vaciar worker, flipar `GLORY_LIVE_WA_*` en
  ingress; rollback con requeue, sin `pending` huérfanos.
- Paso 4 sesión: `Resolver` trait en núcleo, MN lo implementa envolviendo
  `ensure_session ca53feb` (nunca al revés); sin fallback creador — error
  → reintento + DLQ; sin migración masiva de hilos.
- Paso 5 limpieza: solo cuando los 4 pasos llevan 7 días en verde, borrar
  `whatsapp.rs` viejo, `alerta_whatsapp.rs` worker y puente `[foto]/[audio]`.
- No-goals F5: sin cambiar prompt/tools/tablas MN, sin renombrar `wa_a/wa_b`,
  sin tocar QR/deploy, sin sync total.
- DoD F5: `wa_a`+`wa_b` 7 días en verde, acuse ≤6s p95, cero duplicados,
  `usage` cuadra, rollback ensayado antes del paso 5.

## 8. Riesgos → mitigación (matriz)

- Núcleo contaminado con negocio → frontera §5 + 2º consumidor ficticio que falla si ve `clientes`/precios.
- Sesión cliente×canal rompe `agent_*`/F5/F9 → §6b + test `ensure_session` idempotente + migración solo `canal/via/media_ref`.
- N números sin SLO → acuse 6s, 2xx rápido + turno background, `partir_respuesta()`, backpressure y presupuesto STT/media por sesión/día.
- Webhook/`POST /send` inseguro → threat-model: `X-Gateway-Secret` obligatorio, URL firmada con expiración, traversal 404, `via` allowlist por adapter (`wa_a/wa_b` en MN), sin secretos en el crate.
- Escala Baileys/PC → hub in-memory degradado documentado; N sesiones con QR por sesión y anti-eco como defensa, no como diseño.

## 9. Siguiente acción + DoD observable sin código

Esperar autorización para F1. DoD de diseño: contrato webhook OpenAPI
(`numero_destino/remitente/texto/media_url`), SLO acuse 6s, topes
10 MiB/20s/mime allowlist, estados outbox y matriz de arriba verificables
en revisión. Sin código no hay gate Rust/Sentinel que correr.

## 10. Historial al front (alcance 2026-10-01)

- Hoy: el front solo ve hilos de la app (webhook → sesión → hilo).
  Lo nativo del móvil que nunca tocó el gateway no existe en BD.
- F11 import puntual a pedido: eliges número → adaptador `wpp` pide
  historial Baileys (tope 100 msgs/paginado, respeta rate-limit) →
  normaliza → backfill a `agent_messages` con dedup por
  `(canal, baileys_msg_id)` + `Resolver` existente, sin grupos, sin
  disparar IA/outbox/handoff. El front lo muestra como hilo normal.
- No-goal: sync total continuo de todo WhatsApp (coste, privacidad,
  rate-limit Baileys). Solo lo que pidas, cuando lo pidas.

## 11. Herramienta local + AGENTS.md + skill (alcance 2026-10-01)

- Separación: `lib` sin secretos (conversación, sesión, tools, providers,
  nunca `X-Gateway-Secret`/webhook/QR) vs `bin` herramienta local del dueño
  (`X-Gateway-Secret`, QR, `POST /send`, auth admin). Regla 17 lo declara.
  Corre con `CARGO_TARGET_DIR=C:\tmp\glory-target\glory-agent`, sin SSH,
  sin deploy implícito.
- `AGENTS.md` propio (no existe hoy): una línea que es herramienta
  agnóstica de chat+IA (atender, gestionar, automatizar), con comandos
  `cargo fmt/check/clippy/test` + Sentinel.
- Skill futura `atencion-chat` solo contra el `bin`: acciones `enviar`,
  `leer-hilo`, `tomar/soltar`, `devolver-a-ia`, `import-puntual`; cada una
  con endpoint, ejemplo y precondición (1:1, sin grupos, sin spam,
  `via` allowlist `wa_a/wa_b`).
- DoD: `AGENTS.md` + `SKILL.md` existen y describen lo mismo que §4-§6
  sin contradecirlo; sin código aún.
