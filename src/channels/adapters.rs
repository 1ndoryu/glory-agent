/* Adaptador config-driven (F10/F3): piezas genéricas que cada app usa para
 * montar su adapter de canal sin meter negocio en el núcleo. Réplicas de
 * conducta verificada en MN: `partir_respuesta` (máx 3 partes) y filtro
 * anti-eco de un solo uso (TTL 180s, MAX 500). Sin secretos, sin `Baileys`,
 * sin HTTP: el envío real lo hace el `Sender` que inyecta cada app. */

use std::collections::HashMap;
use std::time::{Duration, Instant};

use chrono::NaiveDate;

/* -------------------------------------------------- partir_respuesta */

/// Máximo de partes por respuesta (paridad con MN `MAX_PARTES = 3`).
pub const MAX_PARTES_RESPUESTA: usize = 3;

/// Parte el texto IA en burbujas: separa por línea en blanco, recorta,
/// descarta vacías y fusiona el exceso en la última parte con espacio.
/// Réplica exacta de `partir_respuesta` de MN para no cambiar el tono.
#[must_use]
pub fn partir_respuesta(texto: &str) -> Vec<String> {
    let partes: Vec<String> = texto
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(str::to_string)
        .collect();
    if partes.len() <= MAX_PARTES_RESPUESTA {
        return partes;
    }
    let mut cortadas = partes[..MAX_PARTES_RESPUESTA - 1].to_vec();
    cortadas.push(partes[MAX_PARTES_RESPUESTA - 1..].join(" "));
    cortadas
}

/* -------------------------------------------------- anti-eco */

/// Clave del filtro: dígitos del número + texto recortado. Claves de menos
/// de 4 caracteres se ignoran (paridad con `eco.mjs` de MN).
#[must_use]
pub fn clave_eco(numero: &str, texto: &str) -> String {
    let digitos: String = numero.chars().filter(char::is_ascii_digit).collect();
    format!("{}|{}", digitos, texto.trim())
}

/// Filtro anti-eco de un solo uso: lo enviado vuelve como inbound en la
/// otra sesión y debe descartarse una vez; si el usuario reenvía el mismo
/// texto después, pasa. Diferencia documentada con `eco.mjs`: MN puede
/// superar `MAX` con entradas vivas; aquí el tope es duro (se expulsa la
/// de expiración más próxima) para acotar memoria con N sesiones.
pub struct EcoFilter {
    ttl: Duration,
    max: usize,
    vistos: HashMap<String, Instant>,
}

impl EcoFilter {
    #[must_use]
    pub fn new(ttl_secs: u64, max: usize) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            max: max.max(1),
            vistos: HashMap::new(),
        }
    }

    /// Registra un envío para que su eco se descarte una vez.
    pub fn registrar(&mut self, destino: &str, texto: &str) {
        let clave = clave_eco(destino, texto);
        if clave.len() < 4 {
            return;
        }
        self.purgar_vencidos();
        if self.vistos.len() >= self.max {
            self.expulsar_mas_proxima();
        }
        self.vistos.insert(clave, Instant::now() + self.ttl);
    }

    /// `true` = es el eco del envío (se consume: una sola vez). Vencido o
    /// desconocido pasa (`false`).
    pub fn es_eco(&mut self, remitente: &str, texto: &str) -> bool {
        let clave = clave_eco(remitente, texto);
        match self.vistos.remove(&clave) {
            None => false,
            Some(expira) => expira >= Instant::now(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.vistos.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vistos.is_empty()
    }

    fn purgar_vencidos(&mut self) {
        let ahora = Instant::now();
        self.vistos.retain(|_, expira| *expira >= ahora);
    }

    fn expulsar_mas_proxima(&mut self) {
        let candidata = self
            .vistos
            .iter()
            .min_by_key(|(_, expira)| **expira)
            .map(|(clave, _)| clave.clone());
        if let Some(clave) = candidata {
            self.vistos.remove(&clave);
        }
    }
}

/* -------------------------------------------------- config */

/// Sesión del adapter: nombre lógico + si la IA responde en ella.
#[derive(Debug, Clone)]
pub struct SesionConfig {
    pub nombre: String,
    pub responde_ia: bool,
}

/// Configuración del adapter de canal (N sesiones, allowlist de `via`,
/// anti-eco y presupuestos diarios por sesión). La app la carga de su
/// config y la valida antes de arrancar; el núcleo no lee ficheros.
#[derive(Debug, Clone)]
pub struct AdapterConfig {
    pub sesiones: Vec<SesionConfig>,
    pub via_permitidas: Vec<String>,
    pub eco_ttl_secs: u64,
    pub eco_max: usize,
    pub stt_seg_dia_por_sesion: u64,
    pub media_bytes_dia_por_sesion: u64,
    pub mensajes_dia_por_sesion: u32,
}

/// Error de configuración del adapter (falla al arrancar, no en caliente).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    SinSesiones,
    SesionDuplicada(String),
    SesionVacia,
    SinVias,
    ViaVacia,
    EcoTtlFueraDeRango,
    EcoMaxFueraDeRango,
    PresupuestoCero,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SinSesiones => write!(f, "el adapter necesita al menos una sesión"),
            Self::SesionDuplicada(n) => write!(f, "sesión duplicada: {n}"),
            Self::SesionVacia => write!(f, "hay una sesión con nombre vacío"),
            Self::SinVias => write!(f, "la allowlist de `via` no puede estar vacía"),
            Self::ViaVacia => write!(f, "hay una `via` vacía en la allowlist"),
            Self::EcoTtlFueraDeRango => write!(f, "eco_ttl_secs debe estar entre 1 y 3600"),
            Self::EcoMaxFueraDeRango => write!(f, "eco_max debe estar entre 1 y 10000"),
            Self::PresupuestoCero => write!(f, "los presupuestos diarios deben ser > 0"),
        }
    }
}

impl std::error::Error for ConfigError {}

impl AdapterConfig {
    /// Valida la config al arrancar el adapter.
    pub fn validar(&self) -> Result<(), ConfigError> {
        if self.sesiones.is_empty() {
            return Err(ConfigError::SinSesiones);
        }
        let mut vistos = std::collections::HashSet::new();
        for sesion in &self.sesiones {
            let nombre = sesion.nombre.trim();
            if nombre.is_empty() {
                return Err(ConfigError::SesionVacia);
            }
            if !vistos.insert(nombre.to_string()) {
                return Err(ConfigError::SesionDuplicada(nombre.to_string()));
            }
        }
        if self.via_permitidas.is_empty() {
            return Err(ConfigError::SinVias);
        }
        if self.via_permitidas.iter().any(|v| v.trim().is_empty()) {
            return Err(ConfigError::ViaVacia);
        }
        if !(1..=3600).contains(&self.eco_ttl_secs) {
            return Err(ConfigError::EcoTtlFueraDeRango);
        }
        if !(1..=10000).contains(&self.eco_max) {
            return Err(ConfigError::EcoMaxFueraDeRango);
        }
        if self.stt_seg_dia_por_sesion == 0
            || self.media_bytes_dia_por_sesion == 0
            || self.mensajes_dia_por_sesion == 0
        {
            return Err(ConfigError::PresupuestoCero);
        }
        Ok(())
    }

    /// ¿La IA responde en esta sesión? `None` = sesión no declarada.
    #[must_use]
    pub fn responde_ia(&self, sesion: &str) -> Option<bool> {
        self.sesiones
            .iter()
            .find(|s| s.nombre == sesion)
            .map(|s| s.responde_ia)
    }

    /// ¿Esta `via` está permitida por el adapter?
    #[must_use]
    pub fn via_permitida(&self, via: &str) -> bool {
        self.via_permitidas.iter().any(|v| v == via)
    }
}

/* -------------------------------------------------- presupuestos */

/// Qué se cobra al presupuesto diario de la sesión (hooks STT/media).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cargo {
    SttSeg(u64),
    MediaBytes(u64),
    Mensaje,
}

/// Gasto acumulado de una sesión en un día.
#[derive(Debug, Clone, Copy, Default)]
pub struct GastoDia {
    pub stt_seg: u64,
    pub media_bytes: u64,
    pub mensajes: u32,
}

/// Presupuestos diarios por sesión: el adapter cobra antes de llamar al
/// `Transcriber`/`MediaStore`/enviar; si no hay saldo, no se gasta.
#[derive(Debug, Default)]
pub struct Presupuestos {
    gastos: HashMap<(String, NaiveDate), GastoDia>,
}

/// Sin saldo en el presupuesto diario de la sesión.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SinSaldo {
    pub sesion: String,
}

impl std::fmt::Display for SinSaldo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "presupuesto diario agotado para {}", self.sesion)
    }
}

impl std::error::Error for SinSaldo {}

impl Presupuestos {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Cobra un cargo si hay saldo; si no, error sin descontar nada.
    pub fn cargar(
        &mut self,
        cfg: &AdapterConfig,
        sesion: &str,
        hoy: NaiveDate,
        cargo: Cargo,
    ) -> Result<(), SinSaldo> {
        let gasto = self.gastos.entry((sesion.to_string(), hoy)).or_default();
        let cabe = match cargo {
            Cargo::SttSeg(s) => gasto.stt_seg.saturating_add(s) <= cfg.stt_seg_dia_por_sesion,
            Cargo::MediaBytes(b) => {
                gasto.media_bytes.saturating_add(b) <= cfg.media_bytes_dia_por_sesion
            }
            Cargo::Mensaje => gasto.mensajes < cfg.mensajes_dia_por_sesion,
        };
        if !cabe {
            return Err(SinSaldo {
                sesion: sesion.to_string(),
            });
        }
        match cargo {
            Cargo::SttSeg(s) => gasto.stt_seg += s,
            Cargo::MediaBytes(b) => gasto.media_bytes += b,
            Cargo::Mensaje => gasto.mensajes += 1,
        }
        Ok(())
    }

    /// Gasto acumulado hoy (para consola/auditoría F4).
    #[must_use]
    pub fn gasto(&self, sesion: &str, hoy: NaiveDate) -> GastoDia {
        self.gastos
            .get(&(sesion.to_string(), hoy))
            .copied()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> AdapterConfig {
        AdapterConfig {
            sesiones: vec![
                SesionConfig {
                    nombre: "wa_a".to_string(),
                    responde_ia: true,
                },
                SesionConfig {
                    nombre: "wa_b".to_string(),
                    responde_ia: false,
                },
            ],
            via_permitidas: vec!["wa_a".to_string(), "wa_b".to_string()],
            eco_ttl_secs: 180,
            eco_max: 500,
            stt_seg_dia_por_sesion: 600,
            media_bytes_dia_por_sesion: 100 * 1024 * 1024,
            mensajes_dia_por_sesion: 1000,
        }
    }

    #[test]
    fn partir_respeta_tres_partes() {
        assert_eq!(partir_respuesta("hola"), vec!["hola".to_string()]);
        assert_eq!(
            partir_respuesta("a\n\n\n\nb"),
            vec!["a".to_string(), "b".to_string()]
        );
        let cuatro = "1\n\n2\n\n3\n\n4";
        assert_eq!(
            partir_respuesta(cuatro),
            vec!["1".to_string(), "2".to_string(), "3 4".to_string()]
        );
    }

    #[test]
    fn eco_un_solo_uso_y_pasa_despues() {
        let mut filtro = EcoFilter::new(180, 500);
        filtro.registrar("+34 611-111-111", "hola");
        assert!(filtro.es_eco("34611111111", "hola"));
        assert!(!filtro.es_eco("34611111111", "hola"), "consumido una vez");
        assert!(!filtro.es_eco("34611111111", "otro"));
        assert!(filtro.is_empty());
    }

    #[test]
    fn eco_ignora_clave_corta() {
        let mut filtro = EcoFilter::new(180, 500);
        filtro.registrar("", "");
        assert!(filtro.is_empty());
    }

    #[test]
    fn eco_tope_duro_expulsa() {
        let mut filtro = EcoFilter::new(180, 2);
        filtro.registrar("111", "a");
        filtro.registrar("222", "b");
        filtro.registrar("333", "c");
        assert_eq!(filtro.len(), 2);
    }

    #[test]
    fn eco_vencido_pasa() {
        let mut filtro = EcoFilter::new(0, 500);
        filtro.registrar("111", "hola");
        std::thread::sleep(Duration::from_millis(5));
        assert!(!filtro.es_eco("111", "hola"));
    }

    #[test]
    fn config_valida_y_mira_sesion() {
        let c = cfg();
        assert!(c.validar().is_ok());
        assert_eq!(c.responde_ia("wa_a"), Some(true));
        assert_eq!(c.responde_ia("wa_b"), Some(false));
        assert_eq!(c.responde_ia("wa_x"), None);
        assert!(c.via_permitida("wa_a"));
        assert!(!c.via_permitida("sms"));
    }

    #[test]
    fn config_rechaza_sesiones_duplicadas() {
        let mut c = cfg();
        c.sesiones.push(SesionConfig {
            nombre: "wa_a".to_string(),
            responde_ia: true,
        });
        assert_eq!(
            c.validar(),
            Err(ConfigError::SesionDuplicada("wa_a".to_string()))
        );
    }

    #[test]
    fn presupuesto_bloquea_sin_saldo() {
        let c = cfg();
        let mut p = Presupuestos::new();
        let hoy = chrono::Local::now().date_naive();
        assert!(p.cargar(&c, "wa_a", hoy, Cargo::Mensaje).is_ok());
        assert_eq!(p.gasto("wa_a", hoy).mensajes, 1);
        let mut chica = cfg();
        chica.mensajes_dia_por_sesion = 1;
        assert_eq!(
            p.cargar(&chica, "wa_a", hoy, Cargo::Mensaje),
            Err(SinSaldo {
                sesion: "wa_a".to_string()
            })
        );
        assert_eq!(p.gasto("wa_a", hoy).mensajes, 1, "no descuenta sin saldo");
    }
}
