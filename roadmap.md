# glory-agent — Roadmap

> Chat realtime + IA reutilizable (Rust). Primer consumidor: Inmobiliaria.
> Referencia: Nakomi (no se toca). Plan: `Agente/planes/plan-glory-agent-2026-09-16.md`.

## Tareas pendientes

- [x] F0 Scaffold lib (169A-1): fmt + check + clippy (`-D warnings`) + 10 unit tests PASS; Sentinel analyze 0 errors / 4 warnings (query_as runtime deliberado, ver `persistence/mod.rs`).
- [x] F1 Transporte (169A-2): 5 tests integración (`tests/ws_roundtrip.rs`) — fanout 2 clientes, aislamiento por sesión, REST sequence sin IA, history vacío sin pool, 429 sobre presupuesto. Total 15 tests PASS; clippy limpio; Sentinel 0 errors.
- [ ] F5 Integración Inmobiliaria: cablear `glory-agent` como dependencia en `MN-Inmobiliaria`, prompts/tools allí, smoke E2E con `OPENCODE_GO_API_KEY`.
- [ ] Migración Nakomi (pendiente explícito, no empezar sin autorización).

## Notas

- Compilar con `CARGO_TARGET_DIR=C:\tmp\glory-target\glory-agent` (techo `C:\tmp` 7 GB).
- Gate: `cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test` + Sentinel.
