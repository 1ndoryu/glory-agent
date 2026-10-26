# atencion-chat — atender conversaciones de chat con IA (glory-agent)

Operar el hilo de una sesión de chat: leerlo, responder como staff, tomarlo o devolverlo a la IA, e importar historial de canal. Plan: `Agente/planes/plan-extraccion-whatsapp-2026-10-01.md` §11.

**Solo contra la herramienta local (`bin`).** Esta skill nunca llama a la lib directamente ni a la BD. Las rutas de consola (`/agent/admin/*`) las monta la app detrás de su auth; la herramienta local es quien las llama.

## Reglas previas (toda acción)

- **1:1 únicamente.** Sin grupos, sin difusiones.
- **Sin spam.** Una respuesta por intervención de staff; nunca en bucle ni en ráfaga.
- **`via` en allowlist:** solo `wa_a` o `wa_b`. Cualquier otro valor se rechaza.
- **El secreto del gateway** (`X-Gateway-Secret`) vive en la herramienta local, nunca en esta skill, en el chat ni en un commit.
- **Sin datos del visitante en `actor`:** el actor es el identificador de staff.

## Acciones

### tomar — staff toma la sesión

- Precondición: sesión en `open` o `escalated`.
- Endpoint: `POST /agent/admin/handoff`
- Cuerpo: `{"session_id": "<uuid>", "accion": "tomar", "actor": "<staff>", "motivo": "<opcional>"}`
- Efecto: `open` → `escalated`. Registra `handoff.tomar`.

### soltar / devolver-a-ia — la IA vuelve a atender

- Precondición: sesión en `escalated`.
- Endpoint: `POST /agent/admin/handoff`
- Cuerpo: `{"session_id": "<uuid>", "accion": "devolver", "actor": "<staff>"}`
- Efecto: `escalated` → `open`. Registra `handoff.devolver`.
- Nota: `soltar` y `devolver-a-ia` son la misma acción (`devolver`).

### leer-hilo — leer la conversación completa

- Endpoint: `GET /agent/admin/hilo?session_id=<uuid>&limite=<n>`
- `limite`: por defecto 50, tope 100.
- Respuesta: `{"mensajes": [...], "siguiente_antes": <n|null>}`. Los mensajes van en orden cronológico ascendente.
- Página anterior: repite la llamada con `&antes=<siguiente_antes>` hasta que `siguiente_antes` sea `null`.

### enviar — responder como staff

- Precondición: sesión tomada por staff (`escalated`) y `via` en allowlist.
- Endpoint: **pendiente de la herramienta local (`bin`)**: `POST /send`. No existe todavía en este repo; la lib no envía nada por canal. Hasta que exista, esta acción no se ejecuta.

### import-puntual — traer historial anterior de un canal

- Precondición: lo pide la persona usuaria para un número concreto. Nada de sync automático.
- Endpoint: `POST /agent/admin/importar`
- Cuerpo:
  ```json
  {
    "session_id": "<uuid>",
    "actor": "<staff>",
    "canal": "whatsapp",
    "via": "wa_a",
    "mensajes": [
      {"sender": "client", "body": "...", "id_externo": "<id del canal>"},
      {"sender": "staff",  "body": "...", "id_externo": "<id del canal>"}
    ]
  }
  ```
- Límites: de 1 a 100 mensajes por lote. `sender` solo `client` o `staff`. `id_externo` obligatorio.
- Respuesta: `{"insertados": n, "duplicados": m}`. Los duplicados (mismo `(canal, id_externo)`) se descartan.
- Efecto: no dispara IA, outbox ni handoff. Queda auditado como `import.puntual`.
- La obtención del historial desde el canal (Baileys) la hace el adaptador de MN, no esta skill.

## Límites honestos

- Hoy existen en la lib: `handoff`, `leer-hilo` (`hilo`) e `import-puntual` (`importar`).
- `enviar` depende de la herramienta local, que aún no está escrita.
- Las consultas SQL nuevas de F11 no tienen prueba E2E contra PostgreSQL (bloqueo 19AA-6).
