/* Canales agnósticos (F10/F1): tipos + traits estrechos para que
 * cualquier app atienda chat web + WhatsApp con IA sin meter negocio aquí.
 * El núcleo solo ve `NormalizedInbound`/`Outbound`/`MediaRef`; cada
 * producto implementa `Ingress`/`Sender`/`Resolver` (+ `MediaStore`,
 * `Transcriber`, `Describer`) y los inyecta. Sin secretos, sin Baileys,
 * sin tablas de negocio: eso vive en el adapter de cada app.
 * Contratos del plan §6: timeout 20s, tope 10 MiB, mime allowlist,
 * `session_id` en `MediaRef`, `via: String` + allowlist por adapter,
 * `idempotency_key` en `Outbound`, SLO wpp (acuse 6s, anti-eco
 * TTL 180s MAX 500, QR por sesión) como obligación del adapter. */

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::AgentError;

/// Tope de media por mensaje (plan §6/§9: 10 MiB).
pub const MAX_MEDIA_BYTES: u64 = 10 * 1024 * 1024;

/// Timeout de `normalizar`/`enviar` (plan §6: 20s).
pub const INGRESS_TIMEOUT_SECS: u64 = 20;

/// Acuse de lectura `WhatsApp` (plan §6 SLO: 6s).
pub const ACUSE_SECS: u64 = 6;

/// Anti-eco del adapter (plan §6 SLO: TTL 180s, MAX 500).
pub const ANTI_ECO_TTL_SECS: u64 = 180;
pub const ANTI_ECO_MAX: usize = 500;

/// Tope del import puntual F11 (plan §10: 100 msgs/paginado).
pub const IMPORT_TOPE_MSGS: usize = 100;

/// Mimes aceptados para foto (plan §6 allowlist).
pub const ALLOWED_IMAGE_MIME: &[&str] = &["image/jpeg", "image/png", "image/webp"];

/// Mimes aceptados para voz (plan §6 allowlist).
pub const ALLOWED_AUDIO_MIME: &[&str] = &["audio/ogg", "audio/mpeg", "audio/mp4"];

/// ¿El mime de media está permitido (foto o voz)?
#[must_use]
pub fn media_mime_permitido(mime: &str) -> bool {
    let mime = mime.trim().to_ascii_lowercase();
    ALLOWED_IMAGE_MIME.contains(&mime.as_str()) || ALLOWED_AUDIO_MIME.contains(&mime.as_str())
}

/// Valida tamaño de media contra el tope de 10 MiB.
pub fn validar_media_tamano(size: u64) -> Result<(), IngressError> {
    if size > MAX_MEDIA_BYTES {
        return Err(IngressError::DemasiadoGrande { size });
    }
    Ok(())
}

/// Referencia a media guardada (plan §6: sin `bytes` para no hacer OOM
/// con tope 10 MiB × N sesiones; el adapter guarda y sirve URL firmada).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaRef {
    pub id: String,
    pub mime: String,
    pub size: u64,
    /// TTL en segundos de la URL firmada (el adapter la renueva).
    pub url_firmada_ttl_secs: u64,
    pub session_id: Uuid,
}

impl MediaRef {
    /// Falla si el mime no está permitido o el tamaño supera el tope.
    pub fn validar(&self) -> Result<(), IngressError> {
        if !media_mime_permitido(&self.mime) {
            return Err(IngressError::MimeNoPermitido {
                mime: self.mime.clone(),
            });
        }
        validar_media_tamano(self.size)
    }
}

/// Entrada cruda de un canal (el adapter la construye desde su webhook/WS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InRaw {
    pub canal: String,
    pub payload: serde_json::Value,
}

/// Entrada normalizada en memoria (plan §6). Los números viajan en plano
/// SOLO aquí; a BD va el hash HMAC (plan §6 frontera PII, secreto en el
/// `bin`/adapter, nunca en la `lib`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedInbound {
    pub canal: String,
    pub numero_destino: String,
    pub remitente: String,
    pub texto: String,
    pub nombre: Option<String>,
    pub media: Option<MediaRef>,
    pub tipo: String,
    /// Id externo del canal (ej. id `Baileys`) para dedup `(canal, id)`.
    pub id_externo: Option<String>,
}

/// Salida unificada (plan §6). `via` libre + allowlist por adapter
/// (`wa_a`/`wa_b` solo ejemplo MN); `idempotency_key` frena el doble envío.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Outbound {
    pub destino: String,
    pub texto: String,
    pub media_url: Option<String>,
    pub via: String,
    pub idempotency_key: String,
}

impl Outbound {
    /// Falla si el destino/clave están vacíos o el texto supera 8000 chars.
    pub fn validar(&self) -> Result<(), SenderError> {
        if self.destino.trim().is_empty() {
            return Err(SenderError::DestinoVacio);
        }
        if self.via.trim().is_empty() {
            return Err(SenderError::ViaVacia);
        }
        if self.idempotency_key.trim().is_empty() {
            return Err(SenderError::SinIdempotencia);
        }
        if self.texto.trim().is_empty() || self.texto.len() > 8000 {
            return Err(SenderError::TextoInvalido);
        }
        Ok(())
    }
}

/// Clave de idempotencia para inbound: el dedup vive en `(canal, id)`.
#[must_use]
pub fn clave_idempotencia_inbound(canal: &str, id_externo: &str) -> String {
    format!("in|{}|{}", canal.trim(), id_externo.trim())
}

/// Acuse de envío del adapter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub message_id: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum IngressError {
    #[error("payload inválido: {0}")]
    PayloadInvalido(String),
    #[error("mime no permitido: {mime}")]
    MimeNoPermitido { mime: String },
    #[error("media demasiado grande: {size} bytes (tope 10 MiB)")]
    DemasiadoGrande { size: u64 },
    #[error("timeout normalizando (20s)")]
    Timeout,
    #[error("interno: {0}")]
    Interno(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SenderError {
    #[error("destino vacío")]
    DestinoVacio,
    #[error("vía vacía")]
    ViaVacia,
    #[error("falta idempotency_key")]
    SinIdempotencia,
    #[error("texto vacío o >8000 chars")]
    TextoInvalido,
    #[error("timeout enviando (20s)")]
    Timeout,
    #[error("interno: {0}")]
    Interno(String),
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("mime no permitido: {mime}")]
    MimeNoPermitido { mime: String },
    #[error("media demasiado grande: {size} bytes (tope 10 MiB)")]
    DemasiadoGrande { size: u64 },
    #[error("interno: {0}")]
    Interno(String),
}

#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("audio no permitido: {mime}")]
    MimeNoPermitido { mime: String },
    #[error("timeout transcribiendo (20s)")]
    Timeout,
    #[error("interno: {0}")]
    Interno(String),
}

#[derive(Debug, thiserror::Error)]
pub enum VisionError {
    #[error("foto no permitida: {mime}")]
    MimeNoPermitido { mime: String },
    #[error("timeout describiendo (20s)")]
    Timeout,
    #[error("interno: {0}")]
    Interno(String),
}

#[derive(Debug, thiserror::Error)]
pub enum ResolverError {
    #[error("canal desconocido: {canal}")]
    CanalDesconocido { canal: String },
    #[error("interno: {0}")]
    Interno(String),
}

impl From<IngressError> for AgentError {
    fn from(e: IngressError) -> Self {
        match e {
            IngressError::PayloadInvalido(_) | IngressError::MimeNoPermitido { .. } => {
                Self::BadRequest(e.to_string())
            }
            IngressError::DemasiadoGrande { .. } => Self::BadRequest(e.to_string()),
            IngressError::Timeout => Self::RateLimited(e.to_string()),
            IngressError::Interno(_) => Self::Internal(e.to_string()),
        }
    }
}

impl From<SenderError> for AgentError {
    fn from(e: SenderError) -> Self {
        match e {
            SenderError::DestinoVacio
            | SenderError::ViaVacia
            | SenderError::SinIdempotencia
            | SenderError::TextoInvalido => Self::BadRequest(e.to_string()),
            SenderError::Timeout => Self::RateLimited(e.to_string()),
            SenderError::Interno(_) => Self::Internal(e.to_string()),
        }
    }
}

impl From<MediaError> for AgentError {
    fn from(e: MediaError) -> Self {
        match e {
            MediaError::MimeNoPermitido { .. } | MediaError::DemasiadoGrande { .. } => {
                Self::BadRequest(e.to_string())
            }
            MediaError::Interno(_) => Self::Internal(e.to_string()),
        }
    }
}

impl From<SttError> for AgentError {
    fn from(e: SttError) -> Self {
        match e {
            SttError::MimeNoPermitido { .. } => Self::BadRequest(e.to_string()),
            SttError::Timeout => Self::RateLimited(e.to_string()),
            SttError::Interno(_) => Self::Ai(e.to_string()),
        }
    }
}

impl From<VisionError> for AgentError {
    fn from(e: VisionError) -> Self {
        match e {
            VisionError::MimeNoPermitido { .. } => Self::BadRequest(e.to_string()),
            VisionError::Timeout => Self::RateLimited(e.to_string()),
            VisionError::Interno(_) => Self::Ai(e.to_string()),
        }
    }
}

impl From<ResolverError> for AgentError {
    fn from(e: ResolverError) -> Self {
        match e {
            ResolverError::CanalDesconocido { .. } => Self::BadRequest(e.to_string()),
            ResolverError::Interno(_) => Self::Internal(e.to_string()),
        }
    }
}

/* Traits estrechos (futuro en caja como `ToolExecutor`: sin `async-trait`
 * para no añadir dependencias). El producto los implementa e inyecta. */

pub trait Ingress: Send + Sync {
    fn normalizar<'a>(
        &'a self,
        raw: &'a InRaw,
    ) -> Pin<Box<dyn Future<Output = Result<NormalizedInbound, IngressError>> + Send + 'a>>;
}

pub trait Sender: Send + Sync {
    fn enviar<'a>(
        &'a self,
        out: &'a Outbound,
    ) -> Pin<Box<dyn Future<Output = Result<Receipt, SenderError>> + Send + 'a>>;
}

pub trait MediaStore: Send + Sync {
    fn guardar<'a>(
        &'a self,
        bytes: &'a [u8],
        mime: &'a str,
        session_id: Uuid,
    ) -> Pin<Box<dyn Future<Output = Result<MediaRef, MediaError>> + Send + 'a>>;
}

pub trait Transcriber: Send + Sync {
    fn audio_a_texto<'a>(
        &'a self,
        media: &'a MediaRef,
    ) -> Pin<Box<dyn Future<Output = Result<String, SttError>> + Send + 'a>>;
}

pub trait Describer: Send + Sync {
    fn foto_a_texto<'a>(
        &'a self,
        media: &'a MediaRef,
    ) -> Pin<Box<dyn Future<Output = Result<String, VisionError>> + Send + 'a>>;
}

pub trait Resolver: Send + Sync {
    fn sesion_por_canal<'a>(
        &'a self,
        cliente_id: &'a str,
        canal: &'a str,
        numero: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Uuid, ResolverError>> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media_valida() -> MediaRef {
        MediaRef {
            id: "m1".to_string(),
            mime: "image/jpeg".to_string(),
            size: 1024,
            url_firmada_ttl_secs: 3600,
            session_id: Uuid::new_v4(),
        }
    }

    #[test]
    fn mime_allowlist_acepta_foto_y_voz() {
        assert!(media_mime_permitido("image/jpeg"));
        assert!(media_mime_permitido("IMAGE/PNG"));
        assert!(media_mime_permitido("audio/ogg"));
        assert!(!media_mime_permitido("video/mp4"));
        assert!(!media_mime_permitido("application/pdf"));
        assert!(!media_mime_permitido(""));
    }

    #[test]
    fn tamano_corta_en_10mib() {
        assert!(validar_media_tamano(MAX_MEDIA_BYTES).is_ok());
        assert!(validar_media_tamano(MAX_MEDIA_BYTES + 1).is_err());
    }

    #[test]
    fn media_ref_rechaza_mime_y_tamano() {
        assert!(media_valida().validar().is_ok());
        let mut mala = media_valida();
        mala.mime = "video/mp4".to_string();
        assert!(matches!(
            mala.validar(),
            Err(IngressError::MimeNoPermitido { .. })
        ));
        let mut grande = media_valida();
        grande.size = MAX_MEDIA_BYTES + 1;
        assert!(matches!(
            grande.validar(),
            Err(IngressError::DemasiadoGrande { .. })
        ));
    }

    #[test]
    fn outbound_exige_destino_via_clave_y_texto() {
        let base = Outbound {
            destino: "34600000000".to_string(),
            texto: "hola".to_string(),
            media_url: None,
            via: "wa_b".to_string(),
            idempotency_key: "in|whatsapp|abc".to_string(),
        };
        assert!(base.validar().is_ok());
        let sin_destino = Outbound {
            destino: "  ".to_string(),
            ..base.clone()
        };
        assert!(matches!(
            sin_destino.validar(),
            Err(SenderError::DestinoVacio)
        ));
        let sin_clave = Outbound {
            idempotency_key: String::new(),
            ..base.clone()
        };
        assert!(matches!(
            sin_clave.validar(),
            Err(SenderError::SinIdempotencia)
        ));
        let largo = Outbound {
            texto: "x".repeat(8001),
            ..base.clone()
        };
        assert!(matches!(largo.validar(), Err(SenderError::TextoInvalido)));
    }

    #[test]
    fn clave_inbound_es_determinista() {
        assert_eq!(
            clave_idempotencia_inbound("whatsapp", "ABC"),
            "in|whatsapp|ABC"
        );
        assert_ne!(
            clave_idempotencia_inbound("whatsapp", "ABC"),
            clave_idempotencia_inbound("whatsapp", "ABD")
        );
    }

    /* Fakes: el transporte se prueba contra estos dobles sin Baileys. */
    pub struct FakeIngress {
        pub salida: NormalizedInbound,
    }

    impl Ingress for FakeIngress {
        fn normalizar<'a>(
            &'a self,
            _raw: &'a InRaw,
        ) -> Pin<Box<dyn Future<Output = Result<NormalizedInbound, IngressError>> + Send + 'a>>
        {
            Box::pin(async move { Ok(self.salida.clone()) })
        }
    }

    pub struct FakeSender {
        pub enviados: std::sync::Mutex<Vec<Outbound>>,
    }

    impl Sender for FakeSender {
        fn enviar<'a>(
            &'a self,
            out: &'a Outbound,
        ) -> Pin<Box<dyn Future<Output = Result<Receipt, SenderError>> + Send + 'a>> {
            Box::pin(async move {
                out.validar()?;
                self.enviados.lock().expect("lock").push(out.clone());
                Ok(Receipt { message_id: None })
            })
        }
    }

    #[test]
    fn fake_sender_respeta_validacion() {
        let sender = FakeSender {
            enviados: std::sync::Mutex::new(vec![]),
        };
        let bueno = Outbound {
            destino: "34600000000".to_string(),
            texto: "hola".to_string(),
            media_url: None,
            via: "wa_b".to_string(),
            idempotency_key: "k1".to_string(),
        };
        let r = futures::executor::block_on(sender.enviar(&bueno));
        assert!(r.is_ok());
        assert_eq!(sender.enviados.lock().expect("lock").len(), 1);
        let malo = Outbound {
            destino: String::new(),
            ..bueno
        };
        assert!(futures::executor::block_on(sender.enviar(&malo)).is_err());
    }

    #[test]
    fn fake_ingress_devuelve_normalizado() {
        let esperado = NormalizedInbound {
            canal: "whatsapp".to_string(),
            numero_destino: "34600000000".to_string(),
            remitente: "34111111111".to_string(),
            texto: "hola".to_string(),
            nombre: None,
            media: None,
            tipo: "texto".to_string(),
            id_externo: Some("m1".to_string()),
        };
        let ingress = FakeIngress {
            salida: esperado.clone(),
        };
        let raw = InRaw {
            canal: "whatsapp".to_string(),
            payload: serde_json::Value::Null,
        };
        let r = futures::executor::block_on(ingress.normalizar(&raw));
        assert!(r.is_ok());
        assert_eq!(r.expect("ok").id_externo.as_deref(), Some("m1"));
    }
}
