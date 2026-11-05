//! Núcleo de simulação de impacto balístico 3D.
//!
//! Modelo: peridinâmica *bond-based* 3D (ν = 1/4) com plasticidade de
//! ligação (encruamento + amolecimento térmico Johnson-Cook, aquecimento
//! adiabático por trabalho plástico), fratura por estiramento crítico e
//! contato por penalidade. O passo de tempo é explícito e **paralelo** (rayon).
//!
//! Não é um código validado de engenharia: é um modelo didático/qualitativo.
pub mod config;
pub mod geometry;
pub mod material;
pub mod sim;
pub mod sweep;

pub use config::{Config, Design, Layer, Nose, Preset, Projectile};
pub use geometry::Model;
pub use material::{Material, LIBRARY};
pub use sim::{Frame, Outcome, OutcomeKind, Sim};

/// Número de threads do pool paralelo do solver.
pub fn threads() -> usize {
    rayon::current_num_threads()
}
