# glory-agent — Roadmap

> Chat realtime + IA reutilizable (Rust). Primer consumidor: Inmobiliaria.
> Referencia: Nakomi (no se toca). Plan: `Agente/planes/plan-glory-agent-2026-09-16.md`.

## Tareas pendientes

- [x] F0 Scaffold lib (169A-1): fmt + check + clippy (`-D warnings`) + 10 unit tests PASS; Sentinel analyze 0 errors / 4 warnings (query_as runtime deliberado, ver `persistence/mod.rs`).
- [x] F1 Transporte (169A-2): 5 tests integración (`tests/ws_roundtrip.rs`) — fanout 2 clientes, aislamiento por sesión, REST sequence sin IA, history vacío sin pool, 429 sobre presupuesto. Total 15 tests PASS; clippy limpio; Sentinel 0 errors.
- [x] F2 Provider E2E (169A-3): Responses API (`/responses`; chat/completions → 500), header `x-opencode-session` = session_id (sin él → 400 MissingSessionID), modelo `muse-spark-1.3-contributor`. E2E real PASS (`reply: OK`, 3.55s). Total 18 tests PASS + 1 E2E; clippy limpio; Sentinel 0 errors.
- [x] F3 Contexto 30k: truncado + `ai_summary` + system compacto, con unit tests. Verificado en suites F0-F2.
- [x] F4 Persistencia (código): repos `create_session/insert_message/list_messages/enqueue_outbox` + nuevo `upsert/get_response_cycle`, `should_answer_with_ai` (+ `ai_may_answer` pura), `fetch_pending_outbox/mark_outbox` con validación de estados en boundary. Total 21 tests PASS (15 unit + 1 header + 5 WS); clippy limpio; Sentinel 0 errors.
- [ ] F4-live Validación contra Postgres viva: correr `migrations/0001_chat.sql` + roundtrip CRUD en `glory_agent_db`. BLOQUEADO: falta password del superusuario postgres local (scram-sha-256); no se adivina ni se reutiliza el `.env` de otro proyecto.
- [ ] F5 Integración Inmobiliaria: cablear `glory-agent` como dependencia en `MN-Inmobiliaria`, prompts/tools allí, smoke E2E con `OPENCODE_GO_API_KEY`.
- [ ] Migración Nakomi (pendiente explícito, no empezar sin autorización).

## Notas

- Compilar con `CARGO_TARGET_DIR=C:\tmp\glory-target\glory-agent` (techo `C:\tmp` 7 GB).
- Gate: `cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test` + Sentinel.
