# Roadmap — glory-agent

> Chat realtime + IA reutilizable (Rust); primer consumidor Inmobiliaria. Referencia intacta: Nakomi (no se toca).
> Compilar con `CARGO_TARGET_DIR=C:\tmp\glory-target\glory-agent` (techo `C:\tmp` 7 GB).
> Gate: `cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test` + Sentinel.
> Plan maestro: `Agente/planes/plan-glory-agent-2026-09-16.md`.

## Siguiente bloque

- [ ] `19AA-1` — **Cerrar F10 (plataforma agnóstica chat+IA) con gate PASS: consola y adaptadores.**
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

- [ ] `19AA-2` — **Dar al front el historial completo del hilo (F11).** Hecho cuando la app muestra el hilo entero
  y el import puntual nativo funciona a petición; sin sync total (§10 del plan F10).
- [ ] `19AA-3` — **Crear la herramienta local y la skill `atencion-chat` (F12).** Hecho cuando existan el AGENTS.md
  propio y la skill con enviar/leer/tomar/importar (§11 del plan F10).
- [ ] `19AA-5` — **Resolver antes del 2026-11-12 la excepción cargo-audit RUSTSEC-2023-0071 (rsa).** Ver
  `.cargo/audit.toml`. Hecho cuando la entrada se quite (si `sqlx-mysql` sale del lock o llega parche) o se renueve
  con fecha nueva.

- [ ] `19AA-4` — **Migración Nakomi.** Hecho externo: autorización explícita de la usuaria; sin ella no se empieza
  (decidido 2026-10-25, antes sin fecha de revisión).
- [ ] `19AA-6` — **E2E vivo del loop de tools contra PG.** Hecho externo: falta una instancia de prueba en el puerto 5433
  con su `DATABASE_URL`. En la 89 solo hay PostgreSQL en 5432, sin `.env` ni `.pgpass` del proyecto (comprobado
  2026-10-25); la usuaria provisiona la base y la credencial de prueba. Los unit cubren el executor con fake.

## Bloqueos


## Planes activos

- `Agente/planes/plan-extraccion-whatsapp-2026-10-01.md` — `19AA-1` — plataforma agnóstica chat+IA (F10).
- `Agente/planes/plan-glory-agent-2026-09-16.md` — `19AA-1`–`19AA-3` — plan maestro (F0–F12); pendiente el lead de
  prueba de punta a punta de F5.
