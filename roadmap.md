# Roadmap — glory-agent

> Chat realtime + IA reutilizable (Rust); primer consumidor Inmobiliaria. Referencia intacta: Nakomi (no se toca).
> Compilar con `CARGO_TARGET_DIR=C:\tmp\glory-target\glory-agent` (techo `C:\tmp` 7 GB).
> Gate: `cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test` + Sentinel.
> Plan maestro: `Agente/planes/plan-glory-agent-2026-09-16.md`.

## Siguiente bloque

## Pendientes

## Bloqueos

- Bloqueo `19AA-2-MN` — cableado F11 en MN (fuera de este repo). La lib ya expone `GET /agent/admin/hilo` (paginado, `siguiente_antes`) e `POST /agent/admin/importar` (lote ≤100, dedup por `id_externo`). Falta que el adaptador wpp pida el historial a Baileys y llame al import, y que el front muestre el hilo entero. Hecho externo: el trabajo en MN.
- Bloqueo `19AA-3-BIN` — herramienta local `bin` (F12 código): `POST /send` y `X-Gateway-Secret`. Depende del adaptador de envío de MN y de un secreto que vive fuera de este repo. La skill `skills/atencion-chat/SKILL.md` ya describe el contrato; `enviar` no se ejecuta hasta que exista.
- Bloqueo `19AA-4` — migración Nakomi. Hecho externo: autorización explícita de la usuaria (decidido 2026-10-25); sin ella no se empieza.
- Bloqueo `19AA-6` — E2E vivo del loop de tools contra PG. Deuda ligada: `hilo.rs` usa consultas en runtime (waiver de Sentinel con motivo) porque el caché `.sqlx/` necesita BD viva; al tenerla, pasar a `query!`/`query_as!` y retirar el waiver. Falta una instancia de prueba en el puerto 5433 con su `DATABASE_URL`: en la 89 solo hay PostgreSQL en 5432, sin `.env` ni `.pgpass` del proyecto (comprobado 2026-10-25). La usuaria provisiona la base y la credencial de prueba; los unit cubren el executor con fake.
- **`rsa` (RUSTSEC-2023-0071): excepción en `.cargo/audit.toml` renovada hasta 2026-11-26 (decidido 2026-10-26).** Sin parche; `sqlx-mysql` sigue en el lock aunque no se compila. Al vencer: quitar la entrada si sale del lock o llega parche; si no, renovar.

## Planes activos

- `Agente/planes/plan-extraccion-whatsapp-2026-10-01.md` — `19AA-1` — plataforma agnóstica chat+IA (F10).
- `Agente/planes/plan-glory-agent-2026-09-16.md` — `19AA-1`–`19AA-3` — plan maestro (F0–F12); pendiente el lead de
  prueba de punta a punta de F5.
