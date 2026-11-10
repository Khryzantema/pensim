//! Núcleo de simulação de impacto balístico 3D.
//!
//! Modelo: peridinâmica *bond-based* 3D (ν = 1/4) com plasticidade de
//! ligação (encruamento + amolecimento térmico Johnson-Cook, aquecimento
//! adiabático por trabalho plástico), fratura por estiramento crítico e
//! contato por penalidade. O passo de tempo é explícito e **paralelo** (rayon).
//!
//! Não é um código validado de engenharia: é um modelo didático/qualitativo.
pub mod ammo;
pub mod config;
pub mod fem;
pub mod geometry;
pub mod material;
pub mod save;
pub mod sim;
pub mod store;
pub mod sweep;

pub use ammo::{Ammo, AMMO};
pub use config::{available_memory_mb, swap_free_mb, usable_memory_mb, Config, Core, Design, Fill, Layer, Nose, Preset, Projectile, Region, TargetObject, point_in_poly, poly_area};
pub use geometry::Model;
pub use material::{adjust_rha, rha_bhn, Material, LIBRARY};
pub use store::PackedFrame;
pub use sim::{Frame, Outcome, OutcomeKind, Sim};

/// Número de threads do pool paralelo do solver.
pub fn threads() -> usize {
    rayon::current_num_threads()
}

/// Simulador em uso pelo app: o núcleo novo de elementos finitos ou a peridinâmica antiga.
pub enum Solver {
    Fem(Box<fem::run::FemSim>),
    Peri(Box<Sim>),
}

impl Solver {
    pub fn new(cfg: Config) -> Result<Solver, String> {
        if cfg.fem {
            Ok(Solver::Fem(Box::new(fem::run::FemSim::new(cfg)?)))
        } else {
            Ok(Solver::Peri(Box::new(Sim::new(cfg)?)))
        }
    }
    pub fn n_physical(&self) -> usize {
        match self { Solver::Fem(s) => s.n_physical(), Solver::Peri(s) => s.n_physical() }
    }
    pub fn symmetry(&self) -> u8 {
        match self { Solver::Fem(s) => s.symmetry(), Solver::Peri(s) => s.symmetry() }
    }
    pub fn normals(&self) -> Vec<[f32; 3]> {
        match self { Solver::Fem(s) => s.normals(), Solver::Peri(s) => s.normals() }
    }
    pub fn dt(&self) -> f64 {
        match self { Solver::Fem(s) => s.dt(), Solver::Peri(s) => s.dt }
    }
    pub fn t_end(&self) -> f64 {
        match self { Solver::Fem(s) => s.t_end(), Solver::Peri(s) => s.t_end() }
    }
    pub fn plate_back(&self) -> f64 {
        match self { Solver::Fem(s) => s.plate_back(), Solver::Peri(s) => s.plate_back() }
    }
    pub fn part_ids(&self) -> Vec<u8> {
        match self { Solver::Fem(s) => s.part_ids(), Solver::Peri(s) => s.part_ids() }
    }
    pub fn surface(&self) -> Vec<bool> {
        match self { Solver::Fem(s) => s.surface(), Solver::Peri(s) => s.surface() }
    }
    pub fn ref_positions(&self) -> Vec<[f32; 3]> {
        match self { Solver::Fem(s) => s.ref_positions(), Solver::Peri(s) => s.ref_positions() }
    }
    /// Conectividade dos hexaedros (só no núcleo de elementos finitos).
    pub fn hex(&self) -> Option<Vec<[u32; 8]>> {
        match self { Solver::Fem(s) => Some(s.hex().to_vec()), Solver::Peri(_) => None }
    }
    pub fn done(&self) -> bool {
        match self { Solver::Fem(s) => s.done(), Solver::Peri(s) => s.done() }
    }
    pub fn finish_now(&mut self) {
        match self { Solver::Fem(s) => s.finish_now(), Solver::Peri(s) => s.finish_now() }
    }
    pub fn advance_for(&mut self, d: std::time::Duration) {
        match self { Solver::Fem(s) => s.advance_for(d), Solver::Peri(s) => s.advance_for(d) }
    }
    pub fn take_frames(&mut self) -> Vec<PackedFrame> {
        match self { Solver::Fem(s) => s.frames.drain(..).collect(), Solver::Peri(s) => s.frames.drain(..).collect() }
    }
    pub fn outcome(&self) -> Outcome {
        match self { Solver::Fem(s) => s.outcome(), Solver::Peri(s) => s.outcome() }
    }
    pub fn steps(&self) -> u64 {
        match self { Solver::Fem(s) => s.steps(), Solver::Peri(s) => s.step_count }
    }
    pub fn time(&self) -> f64 {
        match self { Solver::Fem(s) => s.time(), Solver::Peri(s) => s.time }
    }
    pub fn progress(&self) -> f64 {
        match self { Solver::Fem(s) => s.progress(), Solver::Peri(s) => s.progress() }
    }
}
