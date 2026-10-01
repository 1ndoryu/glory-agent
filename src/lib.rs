#![deny(clippy::all)]
#![warn(clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

/* glory-agent: chat realtime + IA reutilizable.
 * Referencia (solo lectura): NAKOMI/src (agente Claudia). Nakomi no se toca.
 * Primer consumidor: Inmobiliaria. Capas: glory-rs (base) <- glory-agent <- producto.
 * El negocio (prompts y tools concretas) vive en cada consumidor, nunca aquí. */

pub mod channels;
pub mod context;
pub mod errors;
pub mod models;
pub mod persistence;
pub mod prompts;
pub mod providers;
pub mod session;
pub mod timing;
pub mod tools;
pub mod transport;
