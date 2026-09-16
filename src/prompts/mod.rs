/* Prompts por producto. El crate aporta la estructura; cada producto define su
 * identidad (Claudia en Nakomi, agente inmobiliario en Inmobiliaria). Compactos
 * para la ventana de 30k. */

#[derive(Debug, Clone)]
pub struct PromptConfig {
    pub agent_name: String,
    pub identity: String,
    pub business_rules: String,
    pub escalation: String,
}

impl PromptConfig {
    #[must_use]
    pub fn new(agent_name: &str, identity: &str, business_rules: &str, escalation: &str) -> Self {
        Self {
            agent_name: agent_name.to_string(),
            identity: identity.to_string(),
            business_rules: business_rules.to_string(),
            escalation: escalation.to_string(),
        }
    }

    /// Ejemplo inmobiliaria: el consumidor real define el suyo.
    #[must_use]
    pub fn inmobiliaria_ejemplo() -> Self {
        Self::new(
            "Asistente Inmobiliario",
            "Eres el asistente IA de la inmobiliaria. Te identificas como IA siempre.",
            "Respondes sobre inmuebles disponibles, captas nombre y contacto antes de agendar, no inventas precios ni direcciones.",
            "Si el visitante pide humano o das 2 respuestas sin resolver, ofrece contacto por WhatsApp/teléfono y marca escalado.",
        )
    }

    #[must_use]
    pub fn build_system_prompt(&self) -> String {
        format!(
            "{}\nReglas de negocio: {}\nEscalado: {}",
            self.identity, self.business_rules, self.escalation
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_mentions_identity() {
        let cfg = PromptConfig::inmobiliaria_ejemplo();
        let sys = cfg.build_system_prompt();
        assert!(sys.contains("IA"));
        assert!(sys.contains("WhatsApp"));
    }
}
