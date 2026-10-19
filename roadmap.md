# Roadmap — glory-agent

> Chat realtime + IA reutilizable (Rust); primer consumidor Inmobiliaria. Referencia intacta: Nakomi (no se toca).
> Compilar con `CARGO_TARGET_DIR=C:\tmp\glory-target\glory-agent` (techo `C:\tmp` 7 GB).
> Gate: `cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test` + Sentinel.
> Plan maestro: `Agente/planes/plan-glory-agent-2026-09-16.md`.

## Siguiente bloque

- [ ] `19AA-1` — **F10 Plataforma agnóstica chat+IA.**
  En ejecución desde 2026-10-01 (autorizado pasar de diseño a código): núcleo + adaptadores wpp/media/stt, outbox,
  handoff, consola. Plan: `Agente/planes/plan-extraccion-whatsapp-2026-10-01.md`. Hecho: gate reparado (pins
  0.7.16/2.2.9) + `src/channels` (Ingress/Sender/MediaRef/Outbound, gate PASS) + `0004_canal.sql` (UNIQUE parcial +
  `mark_outbox` en boundary + `record_inbound` dedup, etapa `live-f2` PASS) + `channels/adapters.rs` (F3:
  `AdapterConfig` validada + `EcoFilter` un solo uso + `partir_respuesta` 3 partes + `Presupuestos` STT/media/mensajes,
  8 unit, gate PASS) + `0005_handoff.sql` + `src/handoff.rs` (F4: máquina pura tomar/devolver/cerrar/reabrir +
  `handoff` atómico con auditoría + `agent_eventos` append-only + `resumen_uso` + `list_config`, 4 unit + 2 vivos PASS,
  gate PASS) + `Resolver` adoptado por MN (2026-10-05, commit MN `d7c247f0`: `CanalResolver` envuelve
  buscar+`ensure_session`+`vincular`; núcleo intacto).

## Pendientes

- [ ] `19AA-2` — **F11 Historial al front.**
  Solo diseño 2026-10-01: hilo completo app sí; import puntual nativo a pedido, sin sync total. Ver §10 del plan F10.
- [ ] `19AA-3` — **F12 Herramienta local + skill.**
  Solo diseño 2026-10-01: AGENTS.md propio + skill atencion-chat (enviar/leer/tomar/import). Ver §11 del plan F10.
- [ ] `19AA-4` — **Migración Nakomi.**
  Pendiente explícito, no empezar sin autorización.
- [ ] `19AA-5` — **Revisar excepción cargo-audit RUSTSEC-2023-0071 (rsa).**
  Vence 2026-11-12. Ver `.cargo/audit.toml`. Quitarla si hay parche o sqlx-mysql sale del lock; si no, renovar con
  fecha nueva.
- [ ] `19AA-6` — **E2E vivo del loop de tools contra PG (heredado de F6).**
  Falta el E2E vivo del executor de tools contra Postgres viva (requiere instancia 5433; los unit cubren el executor
  con fake). Promovido desde el bloque cerrado F6.

## Bloqueos

(ninguno)

## Planes activos

- `Agente/planes/plan-extraccion-whatsapp-2026-10-01.md` — `19AA-1` — plataforma agnóstica chat+IA (F10).
- `Agente/planes/plan-glory-agent-2026-09-16.md` — `19AA-1`–`19AA-3` — plan maestro (F0–F12); pendiente el lead de
  prueba de punta a punta de F5.
