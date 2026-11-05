/// Propriedades de material (SI). A peridinâmica de ligação 3D fixa ν = 1/4.
#[derive(Clone, Debug, PartialEq)]
pub struct Material {
    pub name: &'static str,
    /// Densidade [kg/m³]
    pub rho: f64,
    /// Módulo de Young [Pa]
    pub young: f64,
    /// Escoamento (tração) [Pa]
    pub yield_strength: f64,
    /// Módulo de encruamento linear [Pa]
    pub hardening: f64,
    /// Estiramento de ligação que causa fratura em tração
    pub fail_stretch: f64,
    /// Razão escoamento compressão/tração (frágeis ≫ 1)
    pub comp_factor: f64,
    /// Calor específico [J/kg/K]
    pub cp: f64,
    /// Temperatura de fusão [K]
    pub t_melt: f64,
    /// Sensibilidade à taxa de deformação (Johnson-Cook C): σy × (1 + C·ln(ε̇/ε̇₀)), ε̇₀ = 1/s
    pub rate_c: f64,
}

const fn m(
    name: &'static str, rho: f64, young: f64, ys: f64, h: f64, fail: f64, comp: f64, cp: f64,
    tm: f64, rc: f64,
) -> Material {
    Material { name, rho, young, yield_strength: ys, hardening: h, fail_stretch: fail,
        comp_factor: comp, cp, t_melt: tm, rate_c: rc }
}

pub const T_REF: f64 = 293.0;

pub const LIBRARY: &[Material] = &[
    m("Aço blindagem RHA", 7850.0, 205e9, 1.10e9, 1.0e9, 0.12, 1.0, 477.0, 1800.0, 0.014),
    m("Aço doce 1020", 7850.0, 205e9, 0.35e9, 0.5e9, 0.25, 1.0, 477.0, 1800.0, 0.022),
    m("Alumínio 6061-T6", 2700.0, 69e9, 0.28e9, 0.1e9, 0.18, 1.0, 896.0, 925.0, 0.002),
    m("Titânio Ti-6Al-4V", 4430.0, 114e9, 0.95e9, 0.4e9, 0.08, 1.0, 526.0, 1900.0, 0.014),
    m("Liga de tungstênio", 17600.0, 360e9, 1.20e9, 0.5e9, 0.06, 1.0, 134.0, 1700.0, 0.016),
    m("Cobre", 8960.0, 120e9, 0.09e9, 0.3e9, 0.40, 1.0, 385.0, 1356.0, 0.025),
    m("Chumbo", 11340.0, 16e9, 0.015e9, 0.02e9, 0.50, 1.0, 129.0, 600.0, 0.05),
    m("Concreto", 2400.0, 30e9, 3.0e6, 0.0, 3.0e-4, 15.0, 880.0, 1500.0, 0.0),
    m("Cerâmica Al2O3", 3900.0, 370e9, 0.25e9, 0.0, 8.5e-4, 8.0, 880.0, 2300.0, 0.0),
    m("Urânio empobrecido", 18950.0, 205e9, 1.10e9, 0.6e9, 0.10, 1.0, 117.0, 1405.0, 0.007),
    m("Enchimento (carga/traçante)", 1650.0, 8e9, 0.06e9, 0.0, 0.05, 3.0, 1000.0, 700.0, 0.0),
    m("Aço de projétil AP", 7850.0, 205e9, 1.60e9, 1.0e9, 0.06, 1.0, 477.0, 1800.0, 0.012),
    // WC-Co (metal duro): muito rígido e duro, frágil em tração (ruptura transversal ~2 GPa)
    m("Carboneto de tungstênio (WC-Co)", 15500.0, 580e9, 1.5e9, 0.0, 0.008, 2.6, 200.0, 3000.0, 0.0),
    // camisa de bala (gilding metal 95Cu-5Zn, encruado na estampagem)
    m("Latão/tombak (camisa)", 8860.0, 115e9, 0.30e9, 0.4e9, 0.30, 1.0, 380.0, 1340.0, 0.025),
    // capuz perfurante (APC/APCBC): face dura, mais tenaz que a ponta do corpo; evita que a
    // ponta se estilhace no impacto
    m("Aço de capuz (APC)", 7850.0, 205e9, 1.25e9, 1.2e9, 0.10, 1.0, 477.0, 1800.0, 0.014),
];

impl Material {
    /// Micromódulo 3D (ν = 1/4): c = 18K / (π δ⁴), K = 2E/3 → c = 12E / (π δ⁴).
    pub fn micromodulus(&self, horizon: f64) -> f64 {
        12.0 * self.young / (std::f64::consts::PI * horizon.powi(4))
    }
    pub fn sound_speed(&self) -> f64 {
        (self.young / self.rho).sqrt()
    }
}
