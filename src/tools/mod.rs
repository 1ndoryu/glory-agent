/* Registro de tools (function calling). El crate define el mecanismo; cada
 * producto registra sus tools concretas (inmobiliaria: buscar, lead, visita;
 * Nakomi: las suyas, pendientes). Schemas cortos para no comerse la ventana. */

use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;

use crate::errors::AgentError;

#[derive(Debug, Clone)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

impl ToolDefinition {
    #[must_use]
    pub fn new(name: &str, description: &str, parameters: Value) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            parameters,
        }
    }

    #[must_use]
    pub fn to_openai(&self) -> Value {
        json!({
            "type": "function",
            "function": {
                "name": self.name,
                "description": self.description,
                "parameters": self.parameters,
            }
        })
    }
}

#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, ToolDefinition>,
}

impl ToolRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, def: ToolDefinition) {
        self.tools.insert(def.name.clone(), def);
    }

    #[must_use]
    pub fn definitions(&self) -> Value {
        self.tools.values().map(ToolDefinition::to_openai).collect()
    }

    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

/* [169A-3] Ejecución de tools (F6). El núcleo define el mecanismo (loop en
 * `transport`); cada producto registra su `ToolExecutor` con las tools
 * concretas. Futuro en caja a propósito: sin `async-trait` para no añadir
 * dependencias al crate. `output` debe ser JSON compacto: el transporte lo
 * reinyecta como `function_call_output` y cada token cuenta para la ventana
 * de 30k. Un executor que falla no tumba el turno: el error se devuelve
 * como texto al modelo para que responda degradado. */

/// Contexto de una llamada: de qué conversación viene y con qué BD.
#[derive(Debug, Clone)]
pub struct ToolCtx {
    pub session_id: Uuid,
    pub pool: Option<sqlx::PgPool>,
}

impl ToolCtx {
    #[must_use]
    pub fn new(session_id: Uuid, pool: Option<sqlx::PgPool>) -> Self {
        Self { session_id, pool }
    }
}

pub trait ToolExecutor: Send + Sync {
    fn execute<'a>(
        &'a self,
        name: &'a str,
        args: &'a Value,
        ctx: &'a ToolCtx,
    ) -> Pin<Box<dyn Future<Output = Result<Value, AgentError>> + Send + 'a>>;
}

/// Máximo de vueltas provider→tools→provider por turno. Cota contra loops
/// del modelo (cada vuelta consume ventana y presupuesto de rate-limit).
pub const MAX_TOOL_ITERATIONS: u8 = 3;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_emits_openai_tools() {
        let mut reg = ToolRegistry::new();
        reg.register(ToolDefinition::new(
            "crear_lead",
            "Registra un lead con nombre y contacto",
            json!({"type": "object", "properties": {"nombre": {"type": "string"}}, "required": ["nombre"]}),
        ));
        assert!(reg.contains("crear_lead"));
        let defs = reg.definitions();
        assert_eq!(defs[0]["function"]["name"], "crear_lead");
    }

    /* [169A-3] El executor se puede falsificar sin BD: el loop del
     * transporte se prueba contra este doble. */
    pub struct FakeExecutor {
        pub calls: std::sync::Mutex<Vec<String>>,
    }

    impl ToolExecutor for FakeExecutor {
        fn execute<'a>(
            &'a self,
            name: &'a str,
            _args: &'a Value,
            _ctx: &'a ToolCtx,
        ) -> Pin<Box<dyn Future<Output = Result<Value, AgentError>> + Send + 'a>> {
            self.calls.lock().expect("lock").push(name.to_string());
            Box::pin(async move { Ok(json!({"ok": true, "tool": name})) })
        }
    }

    #[test]
    fn fake_executor_records_calls() {
        let exe = FakeExecutor {
            calls: std::sync::Mutex::new(vec![]),
        };
        let ctx = ToolCtx::new(Uuid::new_v4(), None);
        let out = futures::executor::block_on(exe.execute("ping", &json!({}), &ctx));
        assert!(out.is_ok());
        assert_eq!(exe.calls.lock().expect("lock").as_slice(), ["ping"]);
    }
}
