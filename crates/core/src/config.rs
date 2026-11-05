use crate::material::{Material, LIBRARY};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Nose {
    Flat,
    Conical,
    Ogive,
    Hemispherical,
}

impl Nose {
    pub const ALL: [Nose; 4] = [Nose::Flat, Nose::Conical, Nose::Ogive, Nose::Hemispherical];
    pub fn label(self) -> &'static str {
        match self {
            Nose::Flat => "Plana",
            Nose::Conical => "Cônica",
            Nose::Ogive => "Ogival",
            Nose::Hemispherical => "Hemisférica",
        }
    }
    fn length(self, r: f64) -> f64 {
        match self {
            Nose::Flat => 0.0,
            Nose::Conical => 2.0 * r,
            Nose::Hemispherical => r,
            Nose::Ogive => {
                let rho = 6.0 * r;
                (rho * rho - (rho - r) * (rho - r)).sqrt()
            }
        }
    }
    fn radius(self, r: f64, z: f64) -> f64 {
        let l = self.length(r);
        if z >= l {
            return r;
        }
        match self {
            Nose::Flat => r,
            Nose::Conical => r * z / l,
            Nose::Hemispherical => (r * r - (r - z) * (r - z)).max(0.0).sqrt(),
            Nose::Ogive => {
                let rho = 6.0 * r;
                ((rho * rho - (l - z) * (l - z)).max(0.0)).sqrt() + r - rho
            }
        }
    }
    /// Perfil de revolução (a, r) a partir da ponta, em metros.
    pub fn profile(self, diameter: f64, length: f64) -> Vec<[f64; 2]> {
        let r = diameter / 2.0;
        let l = self.length(r).min(length);
        let mut v = vec![];
        if self == Nose::Flat {
            v.push([0.0, r]);
        } else {
            let n = 10;
            for k in 0..=n {
                let z = l * k as f64 / n as f64;
                v.push([z, self.radius(r, z)]);
            }
        }
        v.push([length, r]);
        v
    }
}

/// Projétil = esboço de perfil (a, r) revolvido em torno do eixo de voo.
#[derive(Clone, Debug, PartialEq)]
pub struct Projectile {
    /// pontos (distância da ponta `a`, raio `r`) em metros, `a` crescente
    pub profile: Vec<[f64; 2]>,
    pub material: Material,
    /// região interna de outro material (núcleo de aço, carga, traçante…)
    pub core: Option<Core>,
}

/// Região interna do projétil; tem prioridade sobre o material externo.
#[derive(Clone, Debug, PartialEq)]
pub struct Core {
    pub profile: Vec<[f64; 2]>,
    pub material: Material,
}

pub fn profile_radius(p: &[[f64; 2]], a: f64) -> f64 {
    let len = p.last().map_or(0.0, |q| q[0]);
    if p.is_empty() || a < p[0][0] || a > len {
        return -1.0;
    }
    for w in p.windows(2) {
        if a <= w[1][0] {
            let d = (w[1][0] - w[0][0]).max(1e-12);
            let t = ((a - w[0][0]) / d).clamp(0.0, 1.0);
            return w[0][1] + t * (w[1][1] - w[0][1]);
        }
    }
    p[p.len() - 1][1]
}

impl Projectile {
    pub fn length(&self) -> f64 {
        self.profile.last().map_or(0.0, |p| p[0])
    }
    pub fn radius_at(&self, a: f64) -> f64 {
        profile_radius(&self.profile, a)
    }
    pub fn max_radius(&self) -> f64 {
        self.profile.iter().map(|p| p[1]).fold(0.0, f64::max)
    }
}

/// Camada do alvo (extrusão de retângulo ou disco ao longo de x).
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub thickness: f64,
    /// folga de ar antes desta camada
    pub gap_before: f64,
    pub width: f64,
    pub height: f64,
    /// disco (diâmetro = `width`) em vez de retângulo
    pub round: bool,
    pub material: Material,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Design {
    pub projectile: Projectile,
    pub layers: Vec<Layer>,
}

impl Design {
    /// Número de peças do projétil (1 ou 2 com núcleo).
    pub fn n_proj_parts(&self) -> usize {
        1 + self.projectile.core.is_some() as usize
    }
    /// Materiais por peça: projétil, núcleo (se houver), camadas do alvo.
    pub fn materials(&self) -> Vec<Material> {
        let mut v = vec![self.projectile.material.clone()];
        if let Some(c) = &self.projectile.core {
            v.push(c.material.clone());
        }
        v.extend(self.layers.iter().map(|l| l.material.clone()));
        v
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    /// Espaçamento dos nós [m]
    pub dx: f64,
    /// Horizonte em múltiplos de dx
    pub horizon_m: f64,
    pub design: Design,
    pub velocity: f64,
    /// Obliquidade (plano x–z) em relação à normal da placa [graus]
    pub obliquity_deg: f64,
    pub clamp_edges: bool,
    pub time_factor: f64,
    pub n_frames: usize,
    /// simula só 1/2 (obliquidade ≠ 0) ou 1/4 (impacto normal) do modelo, com nós-fantasma
    pub symmetry: bool,
}

impl Default for Config {
    fn default() -> Self {
        Preset::Rifle762.config()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preset {
    Rifle762,
    Pistol9,
    Fsp,
    LongRod,
    Laminate,
}

impl Preset {
    pub const ALL: [Preset; 5] =
        [Preset::Rifle762, Preset::Pistol9, Preset::Fsp, Preset::LongRod, Preset::Laminate];
    pub fn label(self) -> &'static str {
        match self {
            Preset::Rifle762 => "7,62 mm núcleo de aço → RHA",
            Preset::Pistol9 => "9 mm chumbo → alumínio",
            Preset::Fsp => "FSP 12,7 mm → aço doce",
            Preset::LongRod => "Penetrador de tungstênio → RHA",
            Preset::Laminate => "7,62 mm → cerâmica + alumínio",
        }
    }
    pub fn config(self) -> Config {
        let mat = |i: usize| LIBRARY[i].clone();
        let layer = |t: f64, w: f64, m: usize| Layer {
            thickness: t, gap_before: 0.0, width: w, height: w, round: false, material: mat(m),
        };
        let proj = |nose: Nose, d: f64, l: f64, m: usize| Projectile {
            profile: nose.profile(d, l), material: mat(m), core: None,
        };
        let (projectile, layers, velocity) = match self {
            Preset::Rifle762 => (proj(Nose::Ogive, 0.00762, 0.028, 0), vec![layer(0.016, 0.06, 0)], 800.0),
            Preset::Pistol9 => (proj(Nose::Hemispherical, 0.009, 0.015, 6), vec![layer(0.008, 0.06, 2)], 380.0),
            Preset::Fsp => (proj(Nose::Flat, 0.0127, 0.0127, 0), vec![layer(0.015, 0.07, 1)], 1000.0),
            Preset::LongRod => (proj(Nose::Flat, 0.008, 0.08, 4), vec![layer(0.03, 0.07, 0)], 1500.0),
            Preset::Laminate => (
                proj(Nose::Ogive, 0.00762, 0.028, 0),
                vec![layer(0.008, 0.06, 8), layer(0.008, 0.06, 2)],
                700.0,
            ),
        };
        Config {
            dx: 0.0015,
            horizon_m: 2.5,
            design: Design { projectile, layers },
            velocity,
            obliquity_deg: 0.0,
            clamp_edges: true,
            time_factor: 1.0,
            n_frames: 100,
            symmetry: true,
        }
    }
}

/// Memória disponível (MB) lida de /proc/meminfo (Linux/Android), se possível.
pub fn available_memory_mb() -> Option<f64> {
    let t = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = t.lines().find(|l| l.starts_with("MemAvailable:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

impl Config {
    /// Número aproximado de nós da malha.
    pub fn estimate_nodes(&self) -> f64 {
        let dx3 = self.dx.powi(3);
        let layers: f64 = self
            .design
            .layers
            .iter()
            .map(|l| {
                let a = if l.round { std::f64::consts::PI / 4.0 * l.width * l.width } else { l.width * l.height };
                a * l.thickness / dx3
            })
            .sum();
        let p = &self.design.projectile;
        let rm = p.max_radius();
        layers + std::f64::consts::PI * rm * rm * p.length() * 0.7 / dx3
    }
    /// Fator de redução de nós por simetria (1, 2 ou 4), já com ~25% de nós-fantasma.
    pub fn sym_gain(&self) -> f64 {
        if !self.symmetry {
            1.0
        } else if self.obliquity_deg.abs() < 1e-9 {
            4.0 / 1.3
        } else {
            2.0 / 1.2
        }
    }
    pub fn bonds_per_node(&self) -> f64 {
        4.19 * self.horizon_m.powi(3)
    }
    /// Memória aproximada (MB): ligações + estado dos nós + quadros gravados.
    pub fn estimate_memory_mb(&self) -> f64 {
        let n = self.estimate_nodes() / self.sym_gain();
        let threads = std::thread::available_parallelism().map_or(4, |v| v.get()) as f64;
        n * (self.bonds_per_node() * 10.0 + 150.0 + 32.0 * threads + 36.0 * self.n_frames as f64) / 1e6
    }
    pub fn obliquity(&self) -> f64 {
        self.obliquity_deg.to_radians()
    }
    pub fn target_thickness(&self) -> f64 {
        self.design.layers.iter().map(|l| l.thickness + l.gap_before).sum()
    }
    /// Tempo final automático [s].
    pub fn t_end(&self) -> f64 {
        let t_eff = self.target_thickness() / self.obliquity().cos().max(0.3);
        (3.0 * (t_eff + self.design.projectile.length()) / self.velocity.max(1.0) * self.time_factor)
            .clamp(40e-6, 3e-3)
    }
}
