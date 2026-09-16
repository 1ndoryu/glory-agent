# glory-agent — Roadmap

> Chat realtime + IA reutilizable (Rust). Primer consumidor: Inmobiliaria.
> Referencia: Nakomi (no se toca). Plan: `Agente/planes/plan-glory-agent-2026-09-16.md`.

## Tareas pendientes

- [ ] F1 Transporte: probar WS con 2 clientes + reconexión history (solo unit tests ahora).
- [ ] F5 Integración Inmobiliaria: cablear `glory-agent` como dependencia en `MN-Inmobiliaria`, prompts/tools allí, smoke E2E con `OPENCODE_GO_API_KEY`.
- [ ] Migración Nakomi (pendiente explícito, no empezar sin autorización).

## Notas

- Compilar con `CARGO_TARGET_DIR=C:\tmp\glory-target\glory-agent` (techo `C:\tmp` 7 GB).
- Gate: `cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test` + Sentinel.
