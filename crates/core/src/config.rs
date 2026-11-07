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
    /// peças internas (camisa, capuz, para-brisa oco, carga, cinta…), aplicadas em ordem:
    /// a última que contém o ponto decide o material. Têm prioridade sobre o núcleo.
    pub regions: Vec<Region>,
}

/// O que preenche uma região do projétil.
#[derive(Clone, Debug, PartialEq)]
pub enum Fill {
    /// vazio (interior do para-brisa balístico, cavidade de ar)
    Void,
    /// material do corpo
    Body,
    Mat(Material),
}

/// Peça do projétil: sólido de revolução entre `inner(a)` e `outer(a)` (raios em função da
/// distância da ponta). `inner` vazio = maciço. Fora do intervalo de `a` do perfil não vale.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub name: String,
    pub outer: Vec<[f64; 2]>,
    pub inner: Vec<[f64; 2]>,
    pub fill: Fill,
    /// contorno desenhado (a, r): se não vazio, vale o polígono em vez de outer/inner
    pub poly: Vec<[f64; 2]>,
}

/// Ponto dentro do polígono (regra par-ímpar).
pub fn point_in_poly(poly: &[[f64; 2]], x: f64, y: f64) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a[1] > y) != (b[1] > y) && x < (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]) + a[0] {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Área com sinal (fórmula do laço).
pub fn poly_area(poly: &[[f64; 2]]) -> f64 {
    let n = poly.len();
    let mut s = 0.0;
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        s += a[0] * b[1] - b[0] * a[1];
    }
    0.5 * s
}

/// Objeto desenhado do alvo: contornos na vista lateral (x = direção de voo a partir da face do
/// alvo, z = vertical) extrudados na largura `width` (eixo y, centrado). Os contornos vêm em
/// ordem de área decrescente: o último que contém o ponto decide (o mais interno vale).
#[derive(Clone, Debug, PartialEq)]
pub struct TargetObject {
    pub name: String,
    pub parts: Vec<(Vec<[f64; 2]>, Fill)>,
    pub width: f64,
}

impl TargetObject {
    /// índice local entre as peças com material (para a numeração de materiais), ou None (vazio/fora)
    pub fn part_at(&self, x: f64, z: f64) -> Option<usize> {
        let mut hit: Option<usize> = None;
        let mut k = 0usize;
        for (poly, fill) in &self.parts {
            let mat = matches!(fill, Fill::Mat(_));
            if point_in_poly(poly, x, z) {
                hit = if mat { Some(k) } else { None };
            }
            if mat {
                k += 1;
            }
        }
        hit
    }
    pub fn materials(&self) -> Vec<Material> {
        self.parts.iter().filter_map(|(_, f)| if let Fill::Mat(m) = f { Some(m.clone()) } else { None }).collect()
    }
    /// (x mín, x máx, z mín, z máx)
    pub fn bbox(&self) -> [f64; 4] {
        let mut b = [f64::MAX, f64::MIN, f64::MAX, f64::MIN];
        for (poly, _) in &self.parts {
            for p in poly {
                b[0] = b[0].min(p[0]);
                b[1] = b[1].max(p[0]);
                b[2] = b[2].min(p[1]);
                b[3] = b[3].max(p[1]);
            }
        }
        b
    }
    pub fn area(&self) -> f64 {
        self.parts.iter().filter(|(_, f)| matches!(f, Fill::Mat(_))).map(|(p, _)| poly_area(p).abs()).fold(0.0, f64::max)
    }
    pub fn scale(&mut self, f: f64) {
        for (poly, _) in self.parts.iter_mut() {
            for q in poly.iter_mut() {
                q[0] *= f;
                q[1] *= f;
            }
        }
        self.width *= f;
    }
}

impl Region {
    /// `min_wall`: cascas mais finas que isso (camisa, para-brisa, cinta) são engrossadas para
    /// dentro até essa espessura, para não sumirem entre os nós da malha.
    pub fn contains(&self, a: f64, rho: f64, min_wall: f64) -> bool {
        if !self.poly.is_empty() {
            return point_in_poly(&self.poly, a, rho);
        }
        let ro = profile_radius(&self.outer, a);
        if ro < 0.0 || rho > ro + 1e-12 {
            return false;
        }
        if self.inner.is_empty() {
            return true;
        }
        let ri = profile_radius(&self.inner, a);
        ri < 0.0 || rho > ri.min(ro - min_wall) + 1e-12
    }
    pub fn scale(&mut self, f: f64) {
        for q in self.outer.iter_mut().chain(self.inner.iter_mut()).chain(self.poly.iter_mut()) {
            q[0] *= f;
            q[1] *= f;
        }
    }
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
    /// Índice da peça (0 = corpo, 1 = núcleo, depois as regiões com material próprio) do ponto
    /// (a, ρ) já dentro do perfil externo; `None` = vazio.
    pub fn part_at(&self, a: f64, rho: f64, min_wall: f64) -> Option<u8> {
        let mut part = 0u8;
        if let Some(c) = &self.core {
            if rho <= profile_radius(&c.profile, a) + 1e-12 {
                part = 1;
            }
        }
        let mut next = 1 + self.core.is_some() as u8;
        // projétil desenhado: fora de todos os contornos é vazio
        let mut void = self.regions.iter().any(|r| !r.poly.is_empty());
        for r in &self.regions {
            let idx = if matches!(r.fill, Fill::Mat(_)) {
                next += 1;
                next - 1
            } else {
                0
            };
            if r.contains(a, rho, min_wall) {
                match r.fill {
                    Fill::Void => void = true,
                    Fill::Body => {
                        void = false;
                        part = 0;
                    }
                    Fill::Mat(_) => {
                        void = false;
                        part = idx;
                    }
                }
            }
        }
        if void { None } else { Some(part) }
    }
    pub fn scale(&mut self, f: f64) {
        for q in self.profile.iter_mut() {
            q[0] *= f;
            q[1] *= f;
        }
        if let Some(c) = self.core.as_mut() {
            for q in c.profile.iter_mut() {
                q[0] *= f;
                q[1] *= f;
            }
        }
        for r in self.regions.iter_mut() {
            r.scale(f);
        }
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
    /// objetos desenhados do alvo (além das chapas)
    pub objects: Vec<TargetObject>,
}

impl Design {
    /// Número de peças do projétil (1 ou 2 com núcleo).
    pub fn n_proj_parts(&self) -> usize {
        1 + self.projectile.core.is_some() as usize + self.projectile.regions.iter().filter(|r| matches!(r.fill, Fill::Mat(_))).count()
    }
    /// Materiais por peça: projétil, núcleo (se houver), camadas do alvo.
    pub fn materials(&self) -> Vec<Material> {
        let mut v = vec![self.projectile.material.clone()];
        if let Some(c) = &self.projectile.core {
            v.push(c.material.clone());
        }
        for r in &self.projectile.regions {
            if let Fill::Mat(m) = &r.fill {
                v.push(m.clone());
            }
        }
        v.extend(self.layers.iter().map(|l| l.material.clone()));
        for o in &self.objects {
            v.extend(o.materials());
        }
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
    /// duração da simulação [s, no tempo do modelo]; 0 = automática
    pub duration: f64,
    pub n_frames: usize,
    /// simula só 1/2 (obliquidade ≠ 0) ou 1/4 (impacto normal) do modelo, com nós-fantasma
    pub symmetry: bool,
    /// região ativa adaptativa: nós em repouso longe do impacto dormem até um vizinho passar de
    /// `sleep_ratio × velocidade de impacto` (0 desliga)
    pub sleep_ratio: f64,
    /// heterogeneidade do material: variação (±) da resistência/ruptura por ligação (0 = ideal)
    pub noise: f64,
    /// perturbação aleatória da posição dos nós, em fração de dx (quebra a anisotropia da malha)
    pub jitter: f64,
    /// ignora a verificação de memória (o usuário escolheu continuar mesmo assim)
    pub ignore_memory: bool,
    /// resistência dinâmica: multiplica o escoamento (e o encruamento) para representar o ganho
    /// por taxa de deformação em impacto balístico (1 = valores quase-estáticos da biblioteca)
    pub rate_gain: f64,
    /// EXPERIMENTAL (desligada): plasticidade só na parte desviadora do estiramento. Ainda não
    /// conserva energia em alta velocidade (erro cresce com v) e não reproduz o lábio de entrada
    pub deviatoric_plasticity: bool,
    /// Peridinâmica "state-based" (Mitchell, Sandia 2011): Poisson real, plasticidade J2 só no
    /// desvio de forma (a pressão fica elástica). Desligado = modelo "bond-based" antigo.
    pub state_based: bool,
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
            profile: nose.profile(d, l), material: mat(m), core: None, regions: vec![],
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
            design: Design { projectile, layers, objects: vec![] },
            velocity,
            obliquity_deg: 0.0,
            clamp_edges: true,
            time_factor: 1.0,
            duration: 0.0,
            n_frames: 100,
            symmetry: true,
            sleep_ratio: 0.05,
            noise: 0.15,
            jitter: 0.05,
            ignore_memory: false,
            rate_gain: 1.0,
            deviatoric_plasticity: false,
            state_based: true,
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

/// Swap livre (MB). No Android é zram (RAM comprimida): ajuda com dados frios, mas a simulação
/// percorre tudo a cada passo, então só contamos uma fração dele.
pub fn swap_free_mb() -> Option<f64> {
    let t = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = t.lines().find(|l| l.starts_with("SwapFree:"))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

/// Memória considerada utilizável: RAM livre + 25% do swap livre.
pub fn usable_memory_mb() -> Option<f64> {
    let ram = available_memory_mb()?;
    Some(ram + 0.25 * swap_free_mb().unwrap_or(0.0))
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
            .sum::<f64>()
            + self.design.objects.iter().map(|o| o.area() * o.width / dx3).sum::<f64>();
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
    /// Memória aproximada (MB) com quadros crus (36 B/nó/quadro).
    pub fn estimate_memory_mb(&self) -> f64 {
        self.estimate_memory_with(false)
    }

    /// Memória aproximada (MB): ligações + estado dos nós + quadros (crus ou comprimidos ≈ 6 B/nó/quadro).
    pub fn estimate_memory_with(&self, compressed: bool) -> f64 {
        let n = self.estimate_nodes() / self.sym_gain();
        let threads = std::thread::available_parallelism().map_or(4, |v| v.get()) as f64;
        let frame_b = if compressed { 6.0 } else { 36.0 };
        n * (self.bonds_per_node() * 12.0 + 150.0 + 16.0 * threads * 4.0 + frame_b * self.n_frames as f64) / 1e6
    }
    pub fn obliquity(&self) -> f64 {
        self.obliquity_deg.to_radians()
    }
    pub fn target_thickness(&self) -> f64 {
        let layers: f64 = self.design.layers.iter().map(|l| l.thickness + l.gap_before).sum();
        let (mut x0, mut x1) = (0.0f64, layers);
        for o in &self.design.objects {
            let b = o.bbox();
            if b[0] <= b[1] {
                x0 = x0.min(b[0]);
                x1 = x1.max(b[1]);
            }
        }
        x1 - x0
    }
    /// Tempo final automático [s].
    pub fn t_end(&self) -> f64 {
        if self.duration > 0.0 {
            return self.duration;
        }
        // tempo MÁXIMO de segurança (o fim normal é quando o projétil anda o comprimento dele + 5× a
        // espessura do alvo): 3× o tempo que levaria em velocidade constante, contando a folga inicial
        let p = &self.design.projectile;
        let dist = 2.0 * p.length() + 5.0 * self.target_thickness() + 2.0 * p.max_radius();
        (3.0 * dist / self.velocity.max(1.0) * self.time_factor).clamp(54e-6, 10e-3)
    }
}
