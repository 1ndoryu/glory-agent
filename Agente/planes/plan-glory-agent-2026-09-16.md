# Plan glory-agent — chat realtime + IA reutilizable (Rust)

Fecha: 2026-09-16 · Estado: activo · Ubicación: `area-trabajo/glory-agent/`
Referencia (solo lectura, no se toca): `NAKOMI/src/` (agente Claudia, funciona bien).
Primer consumidor: Inmobiliaria (`INMOBILIARIA/` front + `MN-Inmobiliaria/` backend).
Nakomi como 2º consumidor queda pendiente.

## 1. Objetivo

Crate Rust `glory-agent`: chat en tiempo real + IA con ventana 30k, reutilizable por
Inmobiliaria y futuros proyectos, sin duplicar la lógica de Nakomi.
Nakomi no se migra ni se modifica en este plan.

## 2. No-goals

- No portar tools de negocio de Nakomi (`ai_tools.rs`, 2601 líneas: hosting, VPS, pagos).
- No bridge multi-instancia (single-instance in-memory en v1).
- No streaming token-a-token en v1 (respuesta completa por WS, como Nakomi hoy).
- No widget TS en v1 (solo backend; el front lo consume por WS/REST).

## 3. Estado verificado (2026-09-16)

- `sentinel` 0.7.4 disponible; `rustc/cargo` 1.95.0 OK.
- `glory-agent/` no existía; creada solo `Agente/planes/` (este archivo).
- Referencia medida: `ai_chat.rs` 888l, `ai_prompts.rs` 508l, `ai_providers.rs` 344l,
  `ai_tools.rs` 2601l, `handlers/chat/` ~2300l en 7 ficheros, `models/chat.rs` 301l.
- `glory-rs/backend` solo tiene hub genérico por `user_id` (máx 8 conn), sin chat
  por sesión ni providers: no basta, justifica el repo nuevo.

## 4. Arquitectura objetivo

```text
glory-agent/
  Cargo.toml          # lib; axum/ws, tokio full, serde/json, sqlx postgres+uuid+chrono,
                      # reqwest rustls, uuid, chrono, thiserror, tracing, dashmap, validator
  src/
    lib.rs            # clippy paranoia (deny all, warn pedantic) como NAKOMI/src/lib.rs:1-15
    errors/           # thiserror; ? en I/O/red/BD; sin unwrap sobre input externo
    models/           # session, message (client|ai|staff|system), response_cycle, outbox
    session/          # ChatHub por session_id + sequence_num monotónico + delivery live/history
    transport/        # ws_visitante + ws_staff + REST fallback + ws_routes()
    providers/        # trait AiProvider OpenAI-compat; impl opencode-go primero
    context/          # ventana 30k: truncado (~4 chars/token), ai_summary, system compacto
    tools/            # trait ToolRegistry (definir + ejecutar); sin negocio dentro
    prompts/          # PromptConfig por producto (system, identidad IA, escalado humano)
    persistence/      # repositories sqlx; SQL solo aquí, prepared (query_as!)
    timing/           # budgets por visitor/IP, semáforo concurrencia, rate-limit
  migrations/
    0001_chat.sql     # sesiones, mensajes, outbox, response_cycle
  tests/              # hub, truncado 30k, provider mock, ciclo mensaje→IA→broadcast
  roadmap.md          # solo trabajo abierto (se crea en F0)
```

Dependencias: `glory-rs (base) <- glory-agent (dominio conversacional) <- producto`.
El negocio (prompts Claudia vs inmobiliaria; tools buscar/lead/visita/WhatsApp) vive
en cada consumidor, nunca en `glory-agent`.

## 5. Decisiones cerradas

1. Repo nuevo `glory-agent` (no dentro de `glory-rs`): evita obligar a sus
   11 consumidores a cargar deps de IA; versionado independiente.
2. Rust, super eficiente: `reqwest::Client` compartido (lección `ai_providers.rs:96-99`),
   WS con sender/receiver separados, broadcast in-memory, `max_tokens` 800 / terse
   para resúmenes, timeouts 15-30s, sin N+1 (CTE/JOIN).
3. Provider inicial: `opencode-go` con `muse-spark-1.3-contribuidor` (OpenAI-compat:
   `baseUrl + Bearer + model`). DeepSeek/Groq/Gemini solo si hacen falta después.
4. Claves solo en server (`OPENCODE_GO_API_KEY`); nunca al front. Rate-limit por
   IP/visitante; PII de leads solo en tablas propias; CORS loopback en dev.
5. Compilación y temporales en `C:\tmp` (`CARGO_TARGET_DIR=C:\tmp\glory-target\glory-agent`).
   Techo `C:\tmp` 7 GB; barrido horario existente.

## 6. Fases

- [ ] **F0 Scaffold**: `cargo new --lib`, `roadmap.md`, `sentinel init --preset rust`,
  baseline PASS, commit inicial. Verificación: `cargo test` + Sentinel PASS.
- [ ] **F1 Núcleo sesión+transporte**: models + ChatHub por sesión + WS
  visitante/staff + REST fallback + tests hub. Verificación: 2 WS reciben `live`,
  reconexión recibe `history` sin duplicados.
- [ ] **F2 Provider opencode-go**: trait + impl + retry simple + body builder + mock
  tests. Verificación: mock 200/429/500 OK.
- [ ] **F3 Contexto 30k**: truncado + resumen + PromptConfig. Verificación: historial
  100 mensajes → request ≤30k tokens estimados.
- [ ] **F4 Persistencia+timing+outbox**: migraciones + repos + budgets +
  response_cycle (toma humana). Verificación: `cargo fmt --check + check + clippy + test`.
- [ ] **F5 Integración Inmobiliaria**: `MN-Inmobiliaria` la consume como dependencia;
  prompts/tools inmobiliaria allí. Verificación: lead de prueba de punta a punta.

## 7. Gate y evidencia por fase

`cargo fmt --check && cargo check && cargo clippy -- -D warnings && cargo test`
+ Sentinel del proyecto. Sin deploy (Coolify solo cuando Inmobiliaria lo pida, vía
`coolify-manager-rs`; SSH prohibido). Cada fase cierra con: tests verdes, diff
revisado, commit coherente, roadmap actualizado.

## 8. Riesgos y mitigación

- Portar de más desde Nakomi y acoplar negocio → F1-F3 sin ninguna tool de negocio;
  `ai_tools.rs` no se copia, solo se define el trait.
- Ventana 30k desbordada → truncado + resumen obligatorio, test de techo.
- Hub in-memory no escala a multi-instancia → aceptado en v1; documentado, no bloquea.

## 9. Siguiente acción

Scaffoldeo F0 (lib + roadmap + Sentinel + commit inicial). Preguntar antes de F1.
