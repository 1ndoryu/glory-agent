/* Handoff F10/F4: máquina pura tomar/devolver/cerrar/reabrir sobre
 * `agent_sessions.status`. Sin BD aquí: `persistence::handoff` la ejecuta
 * en transacción junto al evento de auditoría. Sin PII: `actor` es staff
 * genérico, nunca datos del visitante. */

/// Acciones de consola sobre el hilo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccionHandoff {
    Tomar,
    Devolver,
    Cerrar,
    Reabrir,
}

impl AccionHandoff {
    /// Parsea la acción de la consola (`tomar|devolver|cerrar|reabrir`).
    pub fn parse(s: &str) -> Result<Self, HandoffError> {
        match s.trim() {
            "tomar" => Ok(Self::Tomar),
            "devolver" => Ok(Self::Devolver),
            "cerrar" => Ok(Self::Cerrar),
            "reabrir" => Ok(Self::Reabrir),
            otra => Err(HandoffError::AccionDesconocida(otra.to_string())),
        }
    }

    /// Tipo del evento de auditoría que genera (`agent_eventos.tipo`).
    #[must_use]
    pub const fn tipo_evento(self) -> &'static str {
        match self {
            Self::Tomar => "handoff.tomar",
            Self::Devolver => "handoff.devolver",
            Self::Cerrar => "handoff.cerrar",
            Self::Reabrir => "handoff.reabrir",
        }
    }
}

/// Error de handoff (la consola lo muestra, no es un 500).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandoffError {
    AccionDesconocida(String),
    TransicionInvalida {
        estado: String,
        accion: &'static str,
    },
}

impl std::fmt::Display for HandoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::AccionDesconocida(a) => {
                write!(
                    f,
                    "acción de handoff desconocida: {a} (tomar|devolver|cerrar|reabrir)"
                )
            }
            Self::TransicionInvalida { estado, accion } => {
                write!(f, "no se puede {accion} desde {estado}")
            }
        }
    }
}

impl std::error::Error for HandoffError {}

/// Estado destino de `agent_sessions.status` tras la acción.
/// `tomar`/`cerrar` son idempotentes (repetir es seguro); `devolver`/`reabrir`
/// desde un estado inesperado es error (la consola va desincronizada).
pub fn transicion(
    estado_actual: &str,
    accion: AccionHandoff,
) -> Result<&'static str, HandoffError> {
    match (estado_actual, accion) {
        ("open" | "escalated", AccionHandoff::Tomar) => Ok("escalated"),
        ("escalated", AccionHandoff::Devolver) | ("closed", AccionHandoff::Reabrir) => Ok("open"),
        (_, AccionHandoff::Cerrar) if matches!(estado_actual, "open" | "escalated" | "closed") => {
            Ok("closed")
        }
        _ => Err(HandoffError::TransicionInvalida {
            estado: estado_actual.to_string(),
            accion: match accion {
                AccionHandoff::Tomar => "tomar",
                AccionHandoff::Devolver => "devolver",
                AccionHandoff::Cerrar => "cerrar",
                AccionHandoff::Reabrir => "reabrir",
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tomar_es_idempotente() {
        assert_eq!(transicion("open", AccionHandoff::Tomar), Ok("escalated"));
        assert_eq!(
            transicion("escalated", AccionHandoff::Tomar),
            Ok("escalated")
        );
    }

    #[test]
    fn devolver_solo_desde_humano() {
        assert_eq!(transicion("escalated", AccionHandoff::Devolver), Ok("open"));
        assert!(transicion("open", AccionHandoff::Devolver).is_err());
        assert!(transicion("closed", AccionHandoff::Devolver).is_err());
    }

    #[test]
    fn cerrar_siempre_y_reabrir_solo_cerrado() {
        for estado in ["open", "escalated", "closed"] {
            assert_eq!(transicion(estado, AccionHandoff::Cerrar), Ok("closed"));
        }
        assert_eq!(transicion("closed", AccionHandoff::Reabrir), Ok("open"));
        assert!(transicion("open", AccionHandoff::Reabrir).is_err());
    }

    #[test]
    fn accion_desconocida_falla() {
        assert!(AccionHandoff::parse("quitar").is_err());
        assert_eq!(
            AccionHandoff::parse(" tomar "),
            Ok(AccionHandoff::Tomar),
            "recorta espacios"
        );
    }
}
