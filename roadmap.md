# glory-agent — Roadmap

> Chat realtime + IA reutilizable (Rust). Primer consumidor: Inmobiliaria.
> Referencia: Nakomi (no se toca). Plan: `Agente/planes/plan-glory-agent-2026-09-16.md`.

## Tareas pendientes

- [x] F0 Scaffold lib (169A-1): fmt + check + clippy (`-D warnings`) + 10 unit tests PASS; Sentinel analyze 0 errors / 4 warnings (query_as runtime deliberado, ver `persistence/mod.rs`).
- [x] F1 Transporte (169A-2): 5 tests integración (`tests/ws_roundtrip.rs`) — fanout 2 clientes, aislamiento por sesión, REST sequence sin IA, history vacío sin pool, 429 sobre presupuesto. Total 15 tests PASS; clippy limpio; Sentinel 0 errors.
- [x] F2 Provider E2E (169A-3): Responses API (`/responses`; chat/completions → 500), header `x-opencode-session` = session_id (sin él → 400 MissingSessionID), modelo `muse-spark-1.3-contributor`. E2E real PASS (`reply: OK`, 3.55s). Total 18 tests PASS + 1 E2E; clippy limpio; Sentinel 0 errors.
- [x] F3 Contexto 30k: truncado + `ai_summary` + system compacto, con unit tests. Verificado en suites F0-F2.
- [x] F4 Persistencia (código): repos `create_session/insert_message/list_messages/enqueue_outbox` + nuevo `upsert/get_response_cycle`, `should_answer_with_ai` (+ `ai_may_answer` pura), `fetch_pending_outbox/mark_outbox` con validación de estados en boundary. Total 21 tests PASS (15 unit + 1 header + 5 WS); clippy limpio; Sentinel 0 errors.
- [x] F4-live Validación contra Postgres viva: instancia propia `C:\tmp\pg-glory-agent` (puerto 5433, trust solo loopback; cluster principal intacto, `pg_hba.conf` restaurado hash-verificado). `migrations/0001_chat.sql` aplicada + `tests/db_roundtrip.rs` PASS (sesión→mensajes→UNIQUE→escalado frena IA→outbox→cleanup). Total 21 tests + 2 E2E vivos; clippy limpio; Sentinel 0 errors.
- [x] F5 Integración Inmobiliaria (169A-1, 2026-09-16): `MN-Inmobiliaria`
  usa `glory-agent` por path (rutas `/api/agent/*`, migración `agent_*`,
  PromptConfig propio); widget en `INMOBILIARIA`. Hallazgo y fix aquí:
  `persistence::ensure_session` idempotente llamado en `process_incoming`
  (el transporte acepta `session_id` de cliente; sin bootstrap el primer
  mensaje fallaba por `FK`). Commit `ca53feb`; gate verde.
- [x] F6 Loop de ejecución de tools (169A-3, 2026-09-16): trait
  `tools::ToolExecutor` (futuro en caja, sin deps nuevas) + `ToolCtx` +
  `AgentState::{with_hub,with_executor}` + loop provider→tools→provider
  (máx 3 vueltas, `function_call_output` reinyectado) en `transport`.
  Gate humano real: `ai_may_respond` (`ai_enabled_global`, `ai_enabled`,
  ciclo `escalated`; antes `should_answer_with_ai` nadie la llamaba) +
  `prompt_extra` anexado al system. Persistencia staff/config:
  `get/list/set_session_*`, `get/set_config` + `migrations/0002_config.sql`.
  16 unit + suites verdes; clippy limpio. Falta E2E vivo contra PG
  (requiere instancia 5433; unit cubre executor con fake).
- [ ] Migración Nakomi (pendiente explícito, no empezar sin autorización).

## Notas

- Compilar con `CARGO_TARGET_DIR=C:\tmp\glory-target\glory-agent` (techo `C:\tmp` 7 GB).
- Gate: `cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test` + Sentinel.
