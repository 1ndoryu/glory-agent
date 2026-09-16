/* Registro de tools (function calling). El crate define el mecanismo; cada
 * producto registra sus tools concretas (inmobiliaria: buscar, lead, visita;
 * Nakomi: las suyas, pendientes). Schemas cortos para no comerse la ventana. */

use serde_json::{json, Value};
use std::collections::HashMap;

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
}
