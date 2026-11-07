use crate::config::Config;
use crate::geometry::Model;
use crate::material::{Material, T_REF};
use rayon::prelude::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const TAYLOR_QUINNEY: f64 = 0.9;
/// fração do limite de estabilidade do passo explícito
const DT_SAFETY: f64 = 0.65;

/// Parâmetros de dano dúctil de Johnson-Cook (D1, D2, D3): deformação de ruptura
/// εf = D1 + D2·exp(D3·σ*), σ* = tensão média / von Mises (triaxialidade). None = sem dano dúctil.
fn jc_damage(m: &crate::material::Material) -> Option<(f64, f64, f64)> {
    let n = m.name;
    if m.comp_factor >= 5.0 || n.contains("Enchimento") {
        None
    } else if n.contains("projétil AP") {
        // aço temperado de núcleo/ogiva: pouca ductilidade, trinca e lasca sob cisalhamento/tração
        Some((0.012, 0.16, -1.6))
    } else if n.contains("capuz") {
        Some((0.02, 0.4, -1.6))
    } else if n.contains("Aço doce") {
        Some((0.05, 2.1, -1.0))
    } else if n.contains("Aço") || n.contains("RHA") {
        // 4340 / blindagem (Johnson & Cook 1985)
        Some((0.05, 3.44, -2.12))
    } else if n.contains("Alum") {
        Some((0.05, 1.45, -0.8))
    } else if n.contains("Cobre") || n.contains("Latão") {
        Some((0.54, 4.89, -3.03))
    } else if n.contains("tungst") || n.contains("Urânio") {
        Some((0.03, 0.6, -1.5))
    } else if n.contains("Chumbo") {
        Some((0.5, 3.0, -1.5))
    } else if n.contains("Titân") || n.contains("Ti-6") {
        Some((-0.09, 0.27, -0.48))
    } else {
        None
    }
}

/// Coeficiente de Poisson típico pelo nome do material (o state-based usa o real; o bond-based fixa 1/4).
fn poisson(m: &crate::material::Material) -> f64 {
    let n = m.name;
    if n.contains("Chumbo") { 0.44 } else if n.contains("Concreto") { 0.2 } else if n.contains("Cerâmica") || n.contains("Carboneto") { 0.22 } else if n.contains("Alum") || n.contains("Cobre") || n.contains("Latão") { 0.34 } else if n.contains("Enchimento") { 0.35 } else if n.contains("Textolite") { 0.2 } else if n.contains("tungst") { 0.28 } else { 0.3 }
}
/// 1 / deformação de saturação do encruamento (o ganho máximo é `hardening × HSAT`)
const INV_HSAT: f64 = 1.0 / 0.4;

/// Ligação de peridinâmica (apenas j > i é guardada; cada ligação é calculada uma vez).
#[derive(Clone, Copy)]
struct Bond {
    j: u32,
    /// 1/comprimento de referência; negativo = rompida
    ixi: f32,
    /// correção de volume nas bordas do horizonte
    w: f32,
    sp: f32,
    acc: f32,
    /// fator aleatório fixo da ligação (0..255 → −1..+1): heterogeneidade do material
    rnd: u8,
}

/// Um nó por linha de cache (64 B): posição, amolecimento térmico, temperatura e velocidade.
#[derive(Clone, Copy, Default)]
#[repr(C, align(64))]
struct P {
    x: f64,
    y: f64,
    z: f64,
    soft: f64,
    temp: f64,
    vx: f64,
    vy: f64,
    vz: f64,
}

#[derive(Clone, Copy, Default)]
struct Tot {
    e_el: f64,
    e_ct: f64,
    e_pl: f64,
    e_fr: f64,
}

/// Instantâneo dos campos para visualização (todas as listas têm n nós).
#[derive(Clone, Default)]
pub struct Frame {
    pub time: f64,
    pub pos: Vec<[f32; 3]>,
    pub speed: Vec<f32>,
    pub von_mises: Vec<f32>,
    pub pressure: Vec<f32>,
    pub plastic: Vec<f32>,
    pub temp: Vec<f32>,
    pub damage: Vec<f32>,
    pub ke_proj: f64,
    pub ke_target: f64,
    pub vx_proj: f64,
    pub e_elastic: f64,
    pub e_contact: f64,
    pub e_plastic: f64,
    pub e_fracture: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutcomeKind {
    Perforation,
    PartialPenetration,
    Rejected,
}

impl OutcomeKind {
    pub fn label(self) -> &'static str {
        match self {
            OutcomeKind::Perforation => "Perfuração completa",
            OutcomeKind::PartialPenetration => "Penetração parcial (retido)",
            OutcomeKind::Rejected => "Rejeitado / ricochete",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub kind: OutcomeKind,
    /// velocidade residual do projétil (CM) ao longo de x [m/s]
    pub v_residual: f64,
    pub penetration: f64,
    pub energy_absorbed_pct: f64,
    pub target_ejected_pct: f64,
    pub proj_behind_pct: f64,
    pub energy_error_pct: f64,
}

pub struct Sim {
    pub cfg: Config,
    pub time: f64,
    pub dt: f64,
    pub step_count: u64,
    /// quadros gravados, comprimidos (veja `store`)
    pub frames: Vec<crate::store::PackedFrame>,
    p: Vec<P>,
    /// buffers locais (fx, fy, fz, dT) por bloco de nós: forças acumuladas sem corridas
    locals: Vec<Vec<[f32; 8]>>,
    cur_e_el: f64,
    /// energia elástica volumétrica (state-based)
    cur_e_vol: f64,
    cur_e_ct: f64,
    pos0: Vec<[f64; 3]>,
    part: Vec<u8>,
    /// nós presos na borda do alvo; soltam quando danificados ou sob carga de impacto direto
    fixed: Vec<AtomicBool>,
    surface: Vec<bool>,
    bonds: Vec<Vec<Bond>>,
    initial_bonds: Vec<u32>,
    mats: Vec<Material>,
    c_bond: Vec<f64>,
    c_contact: f64,
    mass: Vec<f64>,
    vol: f64,
    ke0: f64,
    e_pl: f64,
    e_fr: f64,
    next_frame_t: f64,
    t_end: f64,
    back_x: f64,
    cell_start: Vec<u32>,
    cell_items: Vec<u32>,
    cell_of: Vec<u32>,
    grid: GridInfo,
    gmin: [f64; 3],
    gcs: f64,
    ke_hist: Vec<f64>,
    fill: Vec<u32>,
    n_proj: u8,
    /// nós calculados (reais); os demais em `p` são fantasmas espelhados
    n_real: usize,
    /// nós acordados (calculados) e nós rápidos (velocidade acima do limiar)
    awake: Vec<bool>,
    /// velocidade-limiar de despertar [m/s] (0 = região ativa desligada)
    sleep_v: f64,
    /// normais de superfície (apontando para fora) na configuração de referência; ~0 no interior
    normals: Vec<[f32; 3]>,
    /// planos de simetria (bit 1 = y, bit 2 = z)
    sym: u8,
    /// por fantasma: nó real de origem e máscara de espelhamento
    src: Vec<u32>,
    gmask: Vec<u8>,
    /// nós reais perto de um plano de simetria (podem ter contato com fantasmas)
    near: Vec<bool>,
    /// fator de energia física: nº de cópias do nó real no modelo completo
    gfac: f64,
    /// tempos acumulados [s]: grade, forças (ligações+contato), integração
    pub prof: [f64; 3],
    /// tempo da passada prévia do state-based [s]
    pub prof_p1: f64,
    exit_found: bool,
    /// estiramento médio das ligações de cada nó (≈ dilatação/3), do passo anterior
    sbar: Vec<f32>,
    /// --- modelo state-based (valores do passo anterior, por nó real)
    osb: bool,
    /// dilatação θ, volume ponderado m, fator de escoamento do desvio (1 = elástico),
    /// norma² do estado de força desviador final, deformação plástica equivalente e sua taxa
    theta: Vec<f32>,
    mvol: Vec<f32>,
    /// m efetivo do passo (ligações vivas, ≥ 50% do inicial)
    mcur: Vec<f32>,
    /// parte volumétrica espúria das extensões plásticas (aparece quando ligações rompem); é retirada
    thp: Vec<f32>,
    /// dano dúctil acumulado (Johnson-Cook) por nó: 1 = rompe as ligações
    dmg: Vec<f32>,
    jcd: Vec<Option<(f64, f64, f64)>>,
    rsc: Vec<f32>,
    tdn2: Vec<f32>,
    epq: Vec<f32>,
    epr: Vec<f32>,
    /// módulos volumétrico K e de cisalhamento μ por peça
    kbulk: Vec<f64>,
    mu: Vec<f64>,
    hg: HGrid,
    /// nós calculados na região ativa (acordados + vizinhança de ligações)
    active: Vec<bool>,
    halo_done: Vec<bool>,
    /// nós dormindo que receberam força neste passo
    touched: Vec<AtomicBool>,
    /// nós acordados (e fantasmas deles), para atualizar a grade
    awake_items: Vec<u32>,
    awake_count: usize,
    all_awake: bool,
    /// fantasmas do nó real r: [gstart[r], gstart[r+1])
    gstart: Vec<u32>,
    had_contact: bool,
    compress_frames: bool,
    /// instante em que o corpo principal se separou do alvo
    sep_time: Option<f64>,
    /// direção de voo inicial (unitária) e último instante de contato
    dir: [f64; 3],
    last_touch: f64,
    rest_since: Option<f64>,
    /// fim da simulação: posição inicial da traseira, quanto o corpo principal andou e a meta
    rear0: f64,
    travel: f64,
    travel_goal: f64,
    /// intervalo entre quadros [s] (dobra se gravar quadros demais)
    frame_dt: f64,
    captured: usize,
    xcm0: f64,
    x_exit: f64,
}

#[derive(Clone, Copy, Default)]
struct GridInfo {
    n: [i64; 3],
}

/// Espalha os bits de `v` (até 21) para ocupar cada terceira posição.
fn spread3(mut v: u64) -> u64 {
    v &= 0x1f_ffff;
    v = (v | v << 32) & 0x1f00000000ffff;
    v = (v | v << 16) & 0x1f0000ff0000ff;
    v = (v | v << 8) & 0x100f00f00f00f00f;
    v = (v | v << 4) & 0x10c30c30c30c30c3;
    v = (v | v << 2) & 0x1249249249249249;
    v
}

/// Reordena os nós em curva de Morton: vizinhos no espaço ficam próximos na memória,
/// o que melhora muito o uso de cache nos laços de ligação e contato.
fn morton_reorder(m: &mut Model, dx: f64) {
    let n = m.pos.len();
    let mut mn = [f64::MAX; 3];
    for q in &m.pos {
        for k in 0..3 {
            mn[k] = mn[k].min(q[k]);
        }
    }
    let key: Vec<u64> = m
        .pos
        .iter()
        .map(|q| {
            let c = [0, 1, 2].map(|k| ((q[k] - mn[k]) / dx).floor().max(0.0) as u64);
            spread3(c[0]) | spread3(c[1]) << 1 | spread3(c[2]) << 2
        })
        .collect();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_unstable_by_key(|&i| key[i]);
    m.pos = order.iter().map(|&i| m.pos[i]).collect();
    m.part = order.iter().map(|&i| m.part[i]).collect();
    m.fixed = order.iter().map(|&i| m.fixed[i]).collect();
}

const NIL: u32 = u32::MAX;

/// Grade espacial persistente: hash de células de lado `dx` com listas ligadas. Só os nós que
/// mudam de célula são re-ligados (os nós dormindo nunca mudam).
struct HGrid {
    mask: usize,
    head: Vec<u32>,
    next: Vec<u32>,
    cell: Vec<[i32; 3]>,
    inv: f64,
}

impl HGrid {
    fn new(n: usize, dx: f64) -> HGrid {
        let buckets = (2 * n).max(1024).next_power_of_two();
        HGrid { mask: buckets - 1, head: vec![NIL; buckets], next: vec![NIL; n], cell: vec![[0; 3]; n], inv: 1.0 / dx }
    }
    #[inline]
    fn cell_of(&self, x: f64, y: f64, z: f64) -> [i32; 3] {
        [(x * self.inv).floor() as i32, (y * self.inv).floor() as i32, (z * self.inv).floor() as i32]
    }
    #[inline]
    fn bucket(&self, c: [i32; 3]) -> usize {
        let h = (c[0] as u32).wrapping_mul(0x9E37_79B1) ^ (c[1] as u32).wrapping_mul(0x85EB_CA77) ^ (c[2] as u32).wrapping_mul(0xC2B2_AE3D);
        ((h ^ (h >> 15)) as usize) & self.mask
    }
    fn insert(&mut self, i: usize, c: [i32; 3]) {
        let b = self.bucket(c);
        self.next[i] = self.head[b];
        self.head[b] = i as u32;
        self.cell[i] = c;
    }
    fn remove(&mut self, i: usize) {
        let b = self.bucket(self.cell[i]);
        if self.head[b] == i as u32 {
            self.head[b] = self.next[i];
        } else {
            let mut k = self.head[b];
            while k != NIL && self.next[k as usize] != i as u32 {
                k = self.next[k as usize];
            }
            if k != NIL {
                self.next[k as usize] = self.next[i];
            }
        }
    }
    fn relink(&mut self, i: usize, c: [i32; 3]) {
        if self.cell[i] != c {
            self.remove(i);
            self.insert(i, c);
        }
    }
}

/// Ponteiro compartilhável entre threads (acessos sempre em índices disjuntos).
struct SyncPtr(*mut [f32; 8]);
unsafe impl Send for SyncPtr {}

struct SyncPtr1(*mut f32);
unsafe impl Send for SyncPtr1 {}
unsafe impl Sync for SyncPtr1 {}

/// Ponteiros para os vetores por nó do state-based (cada índice é escrito por uma única thread).
struct SyncOsb(*mut f32, *mut f32, *mut f32, *mut f32, *mut f32, *mut f32);
unsafe impl Send for SyncOsb {}
unsafe impl Sync for SyncOsb {}
unsafe impl Sync for SyncPtr {}

/// Todas as 27 células vizinhas.
const ALL27: [(i32, i32, i32); 27] = {
    let mut a = [(0, 0, 0); 27];
    let mut k = 0;
    let mut z = -1;
    while z <= 1 {
        let mut y = -1;
        while y <= 1 {
            let mut x = -1;
            while x <= 1 {
                a[k] = (x, y, z);
                k += 1;
                x += 1;
            }
            y += 1;
        }
        z += 1;
    }
    a
};

/// Deslocamentos de células "à frente": a própria célula + 13 vizinhas (meia vizinhança).
const FORWARD: [(i32, i32, i32); 14] = [
    (0, 0, 0), (1, 0, 0), (-1, 1, 0), (0, 1, 0), (1, 1, 0),
    (-1, -1, 1), (0, -1, 1), (1, -1, 1), (-1, 0, 1), (0, 0, 1), (1, 0, 1),
    (-1, 1, 1), (0, 1, 1), (1, 1, 1),
];

fn cell_coords(c: u32, g: &GridInfo) -> [i64; 3] {
    let c = c as i64;
    [c % g.n[0], (c / g.n[0]) % g.n[1], c / (g.n[0] * g.n[1])]
}

impl Sim {
    pub fn new(cfg: Config) -> Result<Sim, String> {
        // memória: quadros crus por padrão; comprimidos (zstd) só se a simulação estourar a memória
        let mut compress_frames = false;
        if let Some(av) = crate::config::usable_memory_mb() {
            let raw = cfg.estimate_memory_with(false);
            let packed = cfg.estimate_memory_with(true);
            if raw > 0.6 * av {
                compress_frames = true;
            }
            let need = if compress_frames { packed } else { raw };
            if need > 0.85 * av && !cfg.ignore_memory {
                return Err(format!(
                    "Memória insuficiente: esta malha precisa de ≈{:.0} MB (já com compressão) e há ≈{:.0} MB utilizáveis (RAM livre + parte do swap). Aumente o espaçamento, reduza os quadros ou continue mesmo assim.",
                    packed, av
                ));
            }
        }
        let mut model = Model::build(&cfg)?;
        morton_reorder(&mut model, cfg.dx);
        let dx = cfg.dx;
        let vol = dx * dx * dx;
        let horizon = cfg.horizon_m * dx;
        let n_real = model.pos.len();
        let sym = model.sym;
        let masks: Vec<u8> = match sym {
            0 => vec![],
            1 | 2 => vec![sym],
            _ => vec![1, 2, 3],
        };
        let mirror = |q: [f64; 3], m: u8| -> [f64; 3] { [q[0], if m & 1 != 0 { -q[1] } else { q[1] }, if m & 2 != 0 { -q[2] } else { q[2] }] };
        let (mut src, mut gmask) = (vec![], vec![]);
        for r in 0..n_real {
            let q = model.pos[r];
            for &m in &masks {
                let ok_y = m & 1 == 0 || q[1] <= horizon + 1e-12;
                let ok_z = m & 2 == 0 || q[2] <= horizon + 1e-12;
                if ok_y && ok_z {
                    src.push(r as u32);
                    gmask.push(m);
                }
            }
        }
        for (g, &r) in src.iter().enumerate() {
            let q = mirror(model.pos[r as usize], gmask[g]);
            model.pos.push(q);
            let (pt, fx) = (model.part[r as usize], model.fixed[r as usize]);
            model.part.push(pt);
            model.fixed.push(fx);
        }
        let n = model.pos.len();
        let near: Vec<bool> = (0..n_real).map(|i| {
            let q = model.pos[i];
            (sym & 1 != 0 && q[1] <= 2.0 * horizon) || (sym & 2 != 0 && q[2] <= 2.0 * horizon)
        }).collect();
        let gfac = match sym { 0 => 1.0, 3 => 4.0, _ => 2.0 };
        let c_bond: Vec<f64> = model.mats.iter().map(|m| m.micromodulus(horizon)).collect();
        let c_contact = c_bond.iter().cloned().fold(0.0, f64::max);
        let mass: Vec<f64> = model.mats.iter().map(|m| m.rho * vol).collect();
        let speed = cfg.velocity;
        let p: Vec<P> = (0..n)
            .map(|i| {
                let q = model.pos[i];
                let proj = model.part[i] < model.n_proj;
                let sz = if i >= n_real && gmask[i - n_real] & 2 != 0 { -1.0 } else { 1.0 };
                P {
                    x: q[0], y: q[1], z: q[2], temp: T_REF, soft: 1.0,
                    vx: if proj { speed * model.dir[0] } else { 0.0 },
                    vy: 0.0,
                    vz: if proj { sz * speed * model.dir[2] } else { 0.0 },
                }
            })
            .collect();
        let mut sim = Sim {
            time: 0.0,
            dt: 0.0,
            step_count: 0,
            frames: vec![],
            locals: vec![],
            cur_e_el: 0.0,
            cur_e_vol: 0.0,
            cur_e_ct: 0.0,
            p,
            pos0: model.pos,
            part: model.part,
            fixed: model.fixed.iter().map(|&f| AtomicBool::new(f)).collect(),
            surface: vec![false; n_real],
            bonds: vec![vec![]; n_real],
            initial_bonds: vec![0; n_real],
            mats: model.mats,
            c_bond,
            c_contact,
            mass,
            vol,
            ke0: 0.0,
            e_pl: 0.0,
            e_fr: 0.0,
            next_frame_t: 0.0,
            t_end: cfg.t_end(),
            back_x: model.back_x,
            cell_start: vec![],
            cell_items: vec![],
            cell_of: vec![0; n],
            grid: GridInfo::default(),
            gmin: [0.0; 3],
            gcs: 0.0,
            ke_hist: vec![],
            fill: vec![],
            n_proj: model.n_proj,
            n_real,
            awake: vec![true; n_real],
            sleep_v: 0.0,
            normals: vec![[0.0; 3]; n_real],
            sym,
            src,
            gmask,
            near,
            gfac,
            prof: [0.0; 3],
            prof_p1: 0.0,
            exit_found: false,
            sbar: vec![0.0; n_real],
            osb: cfg.state_based,
            theta: vec![0.0; n_real],
            mvol: vec![0.0; n_real],
            mcur: vec![0.0; n_real],
            thp: vec![0.0; n_real],
            dmg: vec![0.0; n_real],
            jcd: vec![],
            rsc: vec![1.0; n_real],
            tdn2: vec![0.0; n_real],
            epq: vec![0.0; n_real],
            epr: vec![0.0; n_real],
            kbulk: vec![],
            mu: vec![],
            hg: HGrid::new(1, 1.0),
            active: vec![false; n_real],
            halo_done: vec![false; n_real],
            touched: (0..n_real).map(|_| AtomicBool::new(false)).collect(),
            awake_items: vec![],
            awake_count: 0,
            all_awake: false,
            gstart: vec![],
            had_contact: false,
            compress_frames,
            sep_time: None,
            dir: model.dir,
            last_touch: 0.0,
            rest_since: None,
            rear0: 0.0,
            travel: 0.0,
            travel_goal: 1.0,
            // o evento real dura ~1/3 do tempo máximo (a simulação termina na separação + 25%)
            frame_dt: cfg.t_end() / (3.0 * cfg.n_frames.max(1) as f64),
            captured: 0,
            xcm0: 0.0,
            x_exit: 1.0,
            cfg,
        };
        sim.build_bonds(horizon);
        // blocos de trabalho: 4× as threads (melhor balanceamento, inclusive em núcleos heterogêneos),
        // limitados pela memória dos buffers locais (n_real × 32 B por bloco)
        let threads = rayon::current_num_threads().max(1);
        let mem_cap = (48_000_000 / (n_real.max(1) * 16)).max(threads);
        let nch = (threads * 4).min(mem_cap).max(1);
        sim.locals = (0..nch).map(|_| vec![[0.0; 8]; n_real]).collect();
        // módulos por peça (ν do material) e volume ponderado inicial m de cada nó
        sim.kbulk = sim.mats.iter().map(|m| m.young / (3.0 * (1.0 - 2.0 * poisson(m)))).collect();
        sim.mu = sim.mats.iter().map(|m| m.young / (2.0 * (1.0 + poisson(m)))).collect();
        sim.jcd = sim.mats.iter().map(jc_damage).collect();
        {
            let nr = sim.n_real;
            let mut mv = vec![0.0f64; nr];
            for i in 0..nr {
                for bd in &sim.bonds[i] {
                    let xi = 1.0 / (bd.ixi as f64).abs();
                    let vj = sim.vol * bd.w as f64;
                    mv[i] += xi * xi * vj;
                    let j = bd.j as usize;
                    let t = if j < nr { j } else { sim.src[j - nr] as usize };
                    if t != i {
                        mv[t] += xi * xi * vj;
                    }
                }
            }
            sim.mvol = mv.iter().map(|&v| v as f32).collect();
            sim.mcur = sim.mvol.clone();
        }
        sim.dt = sim.stable_dt() * if sim.osb { std::env::var("OSBDT").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0) } else { 1.0 };
        sim.ke0 = sim.kinetic(Some(true));
        {
            let (mut sx, mut np) = (0.0, 0.0);
            for i in 0..n_real {
                if sim.part[i] < sim.n_proj {
                    sx += sim.p[i].x;
                    np += 1.0;
                }
            }
            sim.xcm0 = sx / f64::max(np, 1.0);
            let d = sim.dir;
            sim.rear0 = (0..n_real).filter(|&i| sim.part[i] < sim.n_proj).map(|i| sim.p[i].x * d[0] + sim.p[i].z * d[2]).fold(f64::MAX, f64::min);
            sim.travel_goal = sim.cfg.design.projectile.length() + 5.0 * sim.cfg.target_thickness();
            // o corpo sai do alvo quando o centro de massa passa da face traseira + meio comprimento
            sim.x_exit = sim.back_x + 0.5 * sim.cfg.design.projectile.length();
        }
        if sim.cfg.sleep_ratio > 0.0 {
            // só o projétil começa acordado; a placa acorda onde recebe força (contato/ondas)
            sim.sleep_v = sim.cfg.sleep_ratio * sim.cfg.velocity;
            for i in 0..n_real {
                sim.awake[i] = sim.part[i] < sim.n_proj;
            }
        }
        // fantasmas de cada nó real são contíguos (criados na mesma ordem)
        let mut gs = vec![0u32; n_real + 1];
        for &r in &sim.src {
            gs[r as usize + 1] += 1;
        }
        for r in 0..n_real {
            gs[r + 1] += gs[r];
        }
        sim.gstart = gs;
        // grade persistente com todos os nós (reais e fantasmas)
        sim.hg = HGrid::new(n, dx);
        for i in 0..n {
            let q = &sim.p[i];
            let c = sim.hg.cell_of(q.x, q.y, q.z);
            sim.hg.insert(i, c);
        }
        sim.wake_events();
        Ok(sim)
    }

    /// nós calculados (os quadros têm este tamanho)
    /// `true` se os quadros estão sendo guardados comprimidos (zstd).
    pub fn frames_compressed(&self) -> bool {
        self.compress_frames
    }
    pub fn n(&self) -> usize {
        self.n_real
    }
    /// nós do modelo físico completo (≈ nós calculados × cópias por simetria)
    pub fn n_physical(&self) -> usize {
        (self.n_real as f64 * self.gfac) as usize
    }
    /// planos de simetria: bit 1 = y, bit 2 = z
    pub fn symmetry(&self) -> u8 {
        self.sym
    }
    pub fn t_end(&self) -> f64 {
        self.t_end
    }
    /// Progresso (0..1) pela **penetração**: até 80% conforme o centro de massa do projétil
    /// avança até sair do alvo; os 20% finais cobrem a cauda de +25% após a separação.
    /// Se o projétil não avança (retido/ricochete), usa o tempo.
    pub fn progress(&self) -> f64 {
        if self.done() {
            return 1.0;
        }
        // quanto a traseira do corpo principal já andou da distância de fim (ou do tempo máximo)
        let p_tr = (self.travel / self.travel_goal.max(1e-12)).clamp(0.0, 1.0);
        let p_time = (self.time / self.t_end).clamp(0.0, 1.0);
        p_tr.max(p_time).min(0.99)
    }

    /// Eficiência de balanceamento da divisão estática em `nch` blocos: trabalho médio ÷ máximo
    /// (nós ativos por bloco). 1 = perfeito; perto de 0 = quase tudo num só bloco.
    pub fn chunk_balance(&self, nch: usize) -> f64 {
        let nr = self.n_real;
        let per = (nr + nch - 1) / nch;
        let mut w = vec![0usize; nch];
        for i in 0..nr {
            if self.active[i] {
                w[i / per] += self.bonds[i].len().max(1);
            }
        }
        let (sum, mx) = (w.iter().sum::<usize>() as f64, *w.iter().max().unwrap_or(&1) as f64);
        if mx <= 0.0 { 1.0 } else { sum / (nch as f64 * mx) }
    }

    /// (andado, meta) da traseira do corpo principal [m]
    pub fn travel_info(&self) -> (f64, f64) {
        (self.travel, self.travel_goal)
    }

    /// instante (s) em que o corpo principal se separou do alvo, se já ocorreu
    pub fn separation_time(&self) -> Option<f64> {
        self.sep_time
    }

    pub fn done(&self) -> bool {
        self.time >= self.t_end
    }
    pub fn plate_back(&self) -> f64 {
        self.back_x
    }
    /// id da peça de cada nó (0 = projétil)
    pub fn part_ids(&self) -> Vec<u8> {
        self.part[..self.n_real].to_vec()
    }
    /// nós de superfície na configuração de referência
    pub fn surface(&self) -> Vec<bool> {
        self.surface.clone()
    }
    /// normais de superfície dos nós (unitárias; zero no interior)
    pub fn normals(&self) -> Vec<[f32; 3]> {
        self.normals.clone()
    }
    pub fn ref_positions(&self) -> Vec<[f32; 3]> {
        self.pos0[..self.n_real].iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect()
    }

    fn build_grid(&mut self, use_ref: bool, cs_min: f64) {
        let n = self.p.len();
        let get = |i: usize| -> [f64; 3] {
            if use_ref { self.pos0[i] } else { [self.p[i].x, self.p[i].y, self.p[i].z] }
        };
        let small = n < 24_000; // abaixo disso, despachar tarefas paralelas custa mais que o trabalho
        let (mn, mx) = if small {
            let (mut lo, mut hi) = ([f64::MAX; 3], [f64::MIN; 3]);
            for i in 0..n {
                let q = get(i);
                if !q.iter().all(|v| v.is_finite()) {
                    continue;
                }
                for k in 0..3 {
                    lo[k] = lo[k].min(q[k]);
                    hi[k] = hi[k].max(q[k]);
                }
            }
            (lo, hi)
        } else {
            (0..n)
            .into_par_iter()
            .with_min_len(2048)
            .map(|i| get(i))
            .filter(|q| q.iter().all(|v| v.is_finite()))
            .fold(
                || ([f64::MAX; 3], [f64::MIN; 3]),
                |(mut lo, mut hi), q| {
                    for k in 0..3 {
                        lo[k] = lo[k].min(q[k]);
                        hi[k] = hi[k].max(q[k]);
                    }
                    (lo, hi)
                },
            )
            .reduce(
                || ([f64::MAX; 3], [f64::MIN; 3]),
                |(l1, h1), (l2, h2)| ([0, 1, 2].map(|k| l1[k].min(l2[k])), [0, 1, 2].map(|k| h1[k].max(h2[k]))),
            )
        };
        let ext = (0..3).map(|k| mx[k] - mn[k]).fold(cs_min, f64::max);
        let cs = cs_min.max(ext / 90.0);
        let nn = [0, 1, 2].map(|k| ((mx[k] - mn[k]) / cs).floor() as i64 + 1);
        let cells = (nn[0] * nn[1] * nn[2]) as usize;
        let mut cell_of = std::mem::take(&mut self.cell_of);
        let mut cell_start = std::mem::take(&mut self.cell_start);
        let mut cell_items = std::mem::take(&mut self.cell_items);
        let mut fill = std::mem::take(&mut self.fill);
        cell_start.clear();
        cell_start.resize(cells + 1, 0);
        let inv = 1.0 / cs;
        let key = |i: usize| -> u32 {
            let q = get(i);
            let cc = [0, 1, 2].map(|k| (((q[k] - mn[k]) * inv) as i64).clamp(0, nn[k] - 1));
            ((cc[2] * nn[1] + cc[1]) * nn[0] + cc[0]) as u32
        };
        if small {
            for i in 0..n {
                cell_of[i] = key(i);
            }
        } else {
            cell_of.par_iter_mut().with_min_len(2048).enumerate().for_each(|(i, c)| *c = key(i));
        }
        for i in 0..n {
            cell_start[cell_of[i] as usize + 1] += 1;
        }
        for c in 0..cells {
            cell_start[c + 1] += cell_start[c];
        }
        fill.clear();
        fill.extend_from_slice(&cell_start[..cells]);
        cell_items.clear();
        cell_items.resize(n, 0);
        for i in 0..n {
            let c = cell_of[i] as usize;
            cell_items[fill[c] as usize] = i as u32;
            fill[c] += 1;
        }
        self.grid = GridInfo { n: nn };
        self.gmin = mn;
        self.gcs = cs;
        self.cell_of = cell_of;
        self.cell_start = cell_start;
        self.cell_items = cell_items;
        self.fill = fill;
    }

    fn build_bonds(&mut self, horizon: f64) {
        self.build_grid(true, horizon);
        let g = self.grid;
        let dx = self.cfg.dx;
        let nr = self.n_real;
        let full: Vec<(u32, Vec<Bond>, [f64; 3])> = (0..nr)
            .into_par_iter()
            .map(|i| {
                let c = cell_coords(self.cell_of[i], &g);
                let mut list = vec![];
                let mut count = 0u32;
                let mut nsum = [0.0f64; 3];
                for oz in -1..=1 {
                    for oy in -1..=1 {
                        for ox in -1..=1 {
                            let q = [c[0] + ox, c[1] + oy, c[2] + oz];
                            if (0..3).any(|k| q[k] < 0 || q[k] >= g.n[k]) {
                                continue;
                            }
                            let id = ((q[2] * g.n[1] + q[1]) * g.n[0] + q[0]) as usize;
                            for k in self.cell_start[id]..self.cell_start[id + 1] {
                                let j = self.cell_items[k as usize] as usize;
                                if j == i || self.part[j] != self.part[i] {
                                    continue;
                                }
                                let d = [0, 1, 2].map(|a| self.pos0[j][a] - self.pos0[i][a]);
                                let xi = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                                if xi <= horizon + 1e-12 {
                                    let rr = dx / 2.0;
                                    let w = if xi <= horizon - rr {
                                        1.0
                                    } else {
                                        ((horizon + rr - xi) / dx).clamp(0.0, 1.0)
                                    };
                                    count += 1;
                                    for a in 0..3 {
                                        nsum[a] += d[a];
                                    }
                                    // dono único: real–real com j > i; real–fantasma com i ≤ origem do fantasma
                                    let keep = if j < nr { j > i } else { i <= self.src[j - nr] as usize };
                                    if keep {
                                        let (ka, kb, kc) = if j < nr { (i as u64, j as u64, 0u64) } else { (i as u64, self.src[j - nr] as u64, self.gmask[j - nr] as u64 + 1) };
                                        let rnd = (((crate::geometry::Model::hash_unit(7, ka, kb, kc) + 1.0) * 127.5).round() as i64).clamp(0, 255) as u8;
                                        list.push(Bond { j: j as u32, ixi: (1.0 / xi) as f32, w: w as f32, sp: 0.0, acc: 0.0, rnd });
                                    }
                                }
                            }
                        }
                    }
                }
                (count, list, nsum)
            })
            .collect();
        self.initial_bonds = full.iter().map(|b| b.0).collect();
        // normal de superfície: oposta à soma dos vetores para os vizinhos (≈ 0 no interior)
        self.normals = full
            .iter()
            .map(|b| {
                let m = (b.2[0] * b.2[0] + b.2[1] * b.2[1] + b.2[2] * b.2[2]).sqrt();
                if m < 0.15 * dx * (b.0 as f64).max(1.0).sqrt() {
                    [0.0; 3]
                } else {
                    [(-b.2[0] / m) as f32, (-b.2[1] / m) as f32, (-b.2[2] / m) as f32]
                }
            })
            .collect();
        self.bonds = full.into_iter().map(|b| b.1).collect();
        // nós de superfície: bem menos ligações que o interior da peça. A referência é o percentil 90
        // (e não o máximo): com jitter nas posições a contagem varia de nó para nó e o máximo é
        // um outlier que marcaria o interior como superfície.
        let nparts = self.mats.len();
        let mut counts: Vec<Vec<u32>> = vec![vec![]; nparts];
        for i in 0..nr {
            counts[self.part[i] as usize].push(self.initial_bonds[i]);
        }
        let refb: Vec<f64> = counts
            .iter_mut()
            .map(|c| {
                c.sort_unstable();
                c.get(c.len().saturating_mul(9) / 10).copied().unwrap_or(0) as f64
            })
            .collect();
        for i in 0..nr {
            self.surface[i] = (self.initial_bonds[i] as f64) < 0.90 * refb[self.part[i] as usize];
        }
    }

    fn stable_dt(&self) -> f64 {
        let v = self.vol;
        let nr = self.n_real;
        let mut stiff = vec![0.0f64; nr];
        for i in 0..nr {
            let c = self.c_bond[self.part[i] as usize];
            for bd in &self.bonds[i] {
                let k = c * bd.w as f64 * v * bd.ixi as f64;
                stiff[i] += k;
                let j = bd.j as usize;
                let t = if j < nr { j } else { self.src[j - nr] as usize };
                if t != i {
                    stiff[t] += k;
                }
            }
        }
        let mut dt = f64::MAX;
        for i in 0..nr {
            let b = self.part[i] as usize;
            let s = stiff[i] + 12.0 * self.c_contact * v / (0.8 * self.cfg.dx);
            dt = dt.min(DT_SAFETY * (2.0 * self.mats[b].rho / s).sqrt());
        }
        dt
    }

    fn kinetic(&self, proj: Option<bool>) -> f64 {
        self.p[..self.n_real]
            .iter()
            .enumerate()
            .filter(|(i, _)| proj.map_or(true, |pr| (self.part[*i] < self.n_proj) == pr))
            .map(|(i, q)| 0.5 * self.mass[self.part[i] as usize] * (q.vx * q.vx + q.vy * q.vy + q.vz * q.vz))
            .sum()
    }

    /// Nós acordados (fração calculada) — útil para diagnóstico.
    pub fn awake_fraction(&self) -> f64 {
        self.awake.iter().filter(|&&a| a).count() as f64 / self.n_real.max(1) as f64
    }

    /// Registra nós recém-acordados: entram na lista que atualiza a grade e marcam como ativos
    /// (calculados) os vizinhos dentro do horizonte, para que ligações com nós dormindo sejam vistas.
    fn wake_events(&mut self) {
        let nr = self.n_real;
        let new: Vec<usize> = (0..nr).filter(|&i| self.awake[i] && !self.halo_done[i]).collect();
        if new.is_empty() {
            return;
        }
        self.awake_count += new.len();
        self.all_awake = self.awake_count >= nr;
        let everything = self.awake_count * 10 >= nr * 6; // acima de 60% acordado: calcula tudo
        let reach = self.cfg.horizon_m * self.cfg.dx + 1e-9;
        let rng = self.cfg.horizon_m.ceil() as i32;
        let mut mark: Vec<usize> = Vec::new();
        for &i in &new {
            self.halo_done[i] = true;
            self.active[i] = true;
            self.awake_items.push(i as u32);
            if !self.gstart.is_empty() {
                for g in self.gstart[i]..self.gstart[i + 1] {
                    self.awake_items.push((nr as u32) + g);
                }
            }
            if everything {
                continue;
            }
            let c0 = self.hg.cell_of(self.pos0[i][0], self.pos0[i][1], self.pos0[i][2]);
            for oz in -rng..=rng {
                for oy in -rng..=rng {
                    for ox in -rng..=rng {
                        let tc = [c0[0] + ox, c0[1] + oy, c0[2] + oz];
                        let mut cur = self.hg.head[self.hg.bucket(tc)];
                        while cur != NIL {
                            let j = cur as usize;
                            cur = self.hg.next[j];
                            if self.hg.cell[j] != tc || self.part[j] != self.part[i] {
                                continue;
                            }
                            let d2 = (0..3).map(|a| (self.pos0[j][a] - self.pos0[i][a]).powi(2)).sum::<f64>();
                            if d2 <= reach * reach {
                                mark.push(if j < nr { j } else { self.src[j - nr] as usize });
                            }
                        }
                    }
                }
            }
        }
        if everything {
            self.active.iter_mut().for_each(|a| *a = true);
        } else {
            for m in mark {
                self.active[m] = true;
            }
        }
    }

    /// Atualiza fantasmas e células da grade apenas dos nós acordados.
    fn update_awake_grid(&mut self) {
        let nr = self.n_real;
        if !self.src.is_empty() {
            let (real, ghosts) = self.p.split_at_mut(nr);
            let (src, gmask, awake) = (&self.src, &self.gmask, &self.awake);
            ghosts.par_iter_mut().with_min_len(512).enumerate().for_each(|(g, q)| {
                let s = src[g] as usize;
                if !awake[s] {
                    return;
                }
                let r = &real[s];
                let m = gmask[g];
                let sy = if m & 1 != 0 { -1.0 } else { 1.0 };
                let sz = if m & 2 != 0 { -1.0 } else { 1.0 };
                q.x = r.x;
                q.y = sy * r.y;
                q.z = sz * r.z;
                q.vx = r.vx;
                q.vy = sy * r.vy;
                q.vz = sz * r.vz;
                q.temp = r.temp;
                q.soft = r.soft;
            });
        }
        let changed: Vec<(u32, [i32; 3])> = {
            let (p, hg) = (&self.p, &self.hg);
            self.awake_items
                .par_iter()
                .with_min_len(1024)
                .filter_map(|&i| {
                    let q = &p[i as usize];
                    let c = hg.cell_of(q.x, q.y, q.z);
                    if c != hg.cell[i as usize] { Some((i, c)) } else { None }
                })
                .collect()
        };
        for (i, c) in changed {
            self.hg.relink(i as usize, c);
        }
    }

    /// Atualiza os fantasmas a partir dos nós reais (posição, velocidade, temperatura espelhadas).
    fn sync_ghosts(&mut self) {
        if self.src.is_empty() {
            return;
        }
        let nr = self.n_real;
        let (real, ghosts) = self.p.split_at_mut(nr);
        let (src, gmask) = (&self.src, &self.gmask);
        ghosts.par_iter_mut().with_min_len(512).enumerate().for_each(|(g, q)| {
            let r = &real[src[g] as usize];
            let m = gmask[g];
            let sy = if m & 1 != 0 { -1.0 } else { 1.0 };
            let sz = if m & 2 != 0 { -1.0 } else { 1.0 };
            q.x = r.x;
            q.y = sy * r.y;
            q.z = sz * r.z;
            q.vx = r.vx;
            q.vy = sy * r.vy;
            q.vz = sz * r.vz;
            q.temp = r.temp;
            q.soft = r.soft;
        });
    }

    /// Um passo explícito. Cada ligação e cada par em contato é calculado **uma vez** por
    /// blocos de nós em paralelo; as forças vão para buffers locais (sem corridas) e são
    /// somadas na integração. Com simetria, as ligações/contatos com nós-fantasma aplicam a
    /// força no nó real e a reação espelhada no nó real de origem do fantasma.
    pub fn step(&mut self) {
        let dt = self.dt;
        let vol = self.vol;
        let dx = self.cfg.dx;
        let t0 = Instant::now();
        self.update_awake_grid();
        let t1 = Instant::now();
        let nr = self.n_real;
        let nch = self.locals.len();
        let per = (nr + nch - 1) / nch;
        let all_awake = self.all_awake;
        let tp1 = Instant::now();
        if self.osb {
            // state-based, passo 1 (estado ATUAL): por nó, as somas
            //   Sθ = Σ|ξ|e V, S1 = Σ(e−e_p)² V, S2 = Σ(e−e_p)|ξ| V, S3 = Σ|ξ|² V
            // dão a dilatação θ e a norma do desvio de teste sem outra passada:
            //   ‖t_d‖² = α²(S1 − 2θ/3·S2 + θ²/9·S3) → retorno ao escoamento no mesmo passo
            let p = &self.p;
            let src = &self.src;
            let awake = &self.awake;
            let active = &self.active;
            let (dmg, theta0, rsc0, mvol0, part, kbulk0, mu0) = (&self.dmg, &self.theta, &self.rsc, &self.mvol, &self.part, &self.kbulk, &self.mu);
            let noise = self.cfg.noise;
            let gmask = &self.gmask;
            let nobreak = std::env::var("NOBREAK").is_ok();
            let efr: f64 = self.bonds.par_chunks_mut(per).zip(self.locals.par_iter_mut()).enumerate().map(|(c, (bchunk, loc))| -> f64 {
                let base = c * per;
                let mut efr = 0.0f64;
                for (ii, bonds) in bchunk.iter_mut().enumerate() {
                    let i = base + ii;
                    if !active[i] {
                        continue;
                    }
                    let ai = awake[i];
                    let me = &p[i];
                    let mut acc = [0.0f64; 4];
                    let mut acc4 = 0.0f64;
                    for bd in bonds.iter_mut() {
                        let j = bd.j as usize;
                        let jr = if j < nr { j } else { src[j - nr] as usize };
                        if !ai && !awake[jr] {
                            continue;
                        }
                        // dano dúctil (Johnson-Cook): rompe ANTES de calcular θ, para o passo ficar consistente
                        if bd.ixi >= 0.0 {
                            let u = bd.rnd as f64 * (1.0 / 127.5) - 1.0;
                            let dthr = if nobreak { 1e30 } else { 1.0 + 0.3 * noise * u };
                            // média dos dois lados: a trinca se concentra onde o dano é alto dos dois lados
                            // ("qualquer um" soltava nós inteiros e virava uma nuvem de pó)
                            if 0.5 * (dmg[i] as f64 + dmg[jr] as f64) >= dthr {
                                let q = &p[j];
                                let (rx, ry, rz) = (q.x - me.x, q.y - me.y, q.z - me.z);
                                let r = (rx * rx + ry * ry + rz * rz).sqrt();
                                let xi = 1.0 / bd.ixi as f64;
                                let e = r - xi;
                                let vj = vol * bd.w as f64;
                                // energia desviadora perdida (estimada com θ e escoamento do passo anterior)
                                let (bi, bj) = (part[i] as usize, part[j] as usize);
                                let wgt = if j >= nr && jr == i { 0.5 } else { 1.0 };
                                let _ = gmask;
                                for (k, dp, bm) in [(i, bd.sp as f64, bi), (jr, bd.acc as f64, bj)] {
                                    let m0 = (mvol0[k] as f64).max(1e-30);
                                    let al = 15.0 * mu0[bm] / m0;
                                    let td = rsc0[k] as f64 * al * (e - theta0[k] as f64 * xi / 3.0 - dp);
                                    efr += wgt * 0.5 * td * td / al * vol * vj;
                                }
                                let _ = kbulk0;
                                bd.ixi = -bd.ixi;
                                bd.sp = e as f32;
                                bd.acc = 0.0;
                            }
                        }
                        if bd.ixi < 0.0 {
                            // rompida: conta no volume só até a extensão da ruptura (trinca abrindo
                            // não traciona; fechando, ainda comprime) — sem salto de energia na ruptura
                            let q = &p[j];
                            let (rx, ry, rz) = (q.x - me.x, q.y - me.y, q.z - me.z);
                            let r = (rx * rx + ry * ry + rz * rz).sqrt();
                            let xi = -1.0 / bd.ixi as f64;
                            let vj = vol * bd.w as f64;
                            let ec = (r - xi).min(bd.sp as f64);
                            acc[0] += xi * ec * vj;
                            if jr != i {
                                loc[jr][4] += (xi * ec * vj) as f32;
                            }
                            continue;
                        }
                        let q = &p[j];
                        let (rx, ry, rz) = (q.x - me.x, q.y - me.y, q.z - me.z);
                        let r = (rx * rx + ry * ry + rz * rz).sqrt();
                        let xi = 1.0 / bd.ixi as f64;
                        let vj = vol * bd.w as f64;
                        let e = r - xi;
                        let (ui, uj) = (e - bd.sp as f64, e - bd.acc as f64);
                        acc[0] += xi * e * vj;
                        acc[1] += ui * ui * vj;
                        acc[2] += ui * xi * vj;
                        acc[3] += xi * xi * vj;
                        acc4 += bd.sp as f64 * xi * vj;
                        if jr != i {
                            let l = &mut loc[jr];
                            l[3] += (bd.acc as f64 * xi * vj) as f32;
                            l[4] += (xi * e * vj) as f32;
                            l[5] += (uj * uj * vj) as f32;
                            l[6] += (uj * xi * vj) as f32;
                            l[7] += (xi * xi * vj) as f32;
                        }
                    }
                    let l = &mut loc[i];
                    l[3] += acc4 as f32;
                    for k in 0..4 {
                        l[4 + k] += acc[k] as f32;
                    }
                }
                efr
            }).sum();
            self.e_fr += efr;
            let ptrs: Vec<SyncPtr> = self.locals.iter_mut().map(|v| SyncPtr(v.as_mut_ptr())).collect();
            let (awake, active, part, mats, mu) = (&self.awake, &self.active, &self.part, &self.mats, &self.mu);
            let (m0, epq, epr, p) = (&self.mvol, &self.epq, &self.epr, &self.p);
            let rate_gain = self.cfg.rate_gain;
            let frag_frac: f64 = std::env::var("FRAG").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
            let use_thp = std::env::var("THP").map_or(false, |v| v == "1");
            let osbv = SyncOsb(self.theta.as_mut_ptr(), self.mcur.as_mut_ptr(), self.rsc.as_mut_ptr(), self.tdn2.as_mut_ptr(), self.thp.as_mut_ptr(), std::ptr::null_mut());
            let osbv = &osbv;
            (0..nr).into_par_iter().with_min_len(512).for_each(|i| {
                if !active[i] {
                    return;
                }
                let mut a = [0.0f64; 4];
                let mut s4 = 0.0f64;
                for pt in &ptrs {
                    // SAFETY: índice i exclusivo desta thread
                    let v = unsafe { &mut *pt.0.add(i) };
                    s4 += v[3] as f64;
                    v[3] = 0.0;
                    for k in 0..4 {
                        a[k] += v[4 + k] as f64;
                        v[4 + k] = 0.0;
                    }
                }
                // só nós acordados têm todas as ligações somadas; os demais mantêm o estado
                if !awake[i] {
                    return;
                }
                // m das ligações vivas, limitado a ≥ 50% do inicial (nó muito danificado não endurece)
                // m de referência fixo: com o m das ligações vivas, cada ruptura endureceria o nó inteiro
                // (energia criada); a derivada exata (termo D) mantém as forças conservativas
                let me = (m0[i] as f64).max(1e-30);
                let th = 3.0 * a[0] / me;
                // extensões plásticas só desviadoras: tira a parte volumétrica θp (ligações rompidas)
                let thp = if a[3] > 0.0 && use_thp { 3.0 * s4 / a[3] } else { 0.0 };
                let s1 = a[1] + 2.0 * thp / 3.0 * a[2] + thp * thp / 9.0 * a[3];
                let s2 = a[2] + thp / 3.0 * a[3];
                let a = [a[0], s1, s2, a[3]];
                let b = part[i] as usize;
                let m = &mats[b];
                let al = 15.0 * mu[b] / me;
                let n2 = (al * al * (a[1] - 2.0 * th / 3.0 * a[2] + th * th / 9.0 * a[3])).max(0.0);
                let ep = epq[i] as f64;
                let rate = epr[i] as f64;
                let jc = if m.rate_c > 0.0 && rate > 1.0 { 1.0 + m.rate_c * rate.ln() } else { 1.0 };
                let sy = (m.yield_strength + m.hardening * ep / (1.0 + ep * INV_HSAT)) * rate_gain * p[i].soft.max(0.03) * jc;
                // escoamento J2 (Mitchell): ψ0 = 2,5 σy² / m; frágeis não escoam
                // nós muito danificados (fragmentos) ficam elásticos: com poucas ligações o escoamento fica inconsistente
                let damaged = a[3] < frag_frac * m0[i] as f64;
                let r = if m.comp_factor >= 5.0 || n2 <= 0.0 || damaged { 1.0 } else { (5.0 * sy * sy / (me * n2)).sqrt().min(1.0) };
                unsafe {
                    *osbv.0.add(i) = th as f32;
                    *osbv.1.add(i) = me as f32;
                    *osbv.2.add(i) = r as f32;
                    *osbv.3.add(i) = (r * r * n2) as f32;
                    // termo de acoplamento da derivada exata da energia desviadora:
                    // t_k −= α|ξ_k|·D/m, D = Σ(e_d − e_p)|ξ|V = S2 − θ·S3/3
                    let dsum = a[2] - th * a[3] / 3.0;
                    let _ = thp;
                    *osbv.4.add(i) = (al * dsum / me) as f32;
                }
            });
        }
        self.prof_p1 += tp1.elapsed().as_secs_f64();
        let tots: Vec<Tot> = {
            let p = &self.p;
            let part = &self.part;
            let fixed = &self.fixed;
            let mats = &self.mats;
            let c_bond = &self.c_bond;
            let c_ct = self.c_contact;
            let src = &self.src;
            let gmask = &self.gmask;
            let near = &self.near;
            let awake = &self.awake;
            let noise = self.cfg.noise;
            let rate_gain = self.cfg.rate_gain;
            let dev = self.cfg.deviatoric_plasticity;
            let sbar = &self.sbar;
            let osb = self.osb;
            let (theta, mvol, rsc, thp) = (&self.theta, &self.mcur, &self.rsc, &self.thp);
            let dmg = &self.dmg;
            let (kbulk, mu) = (&self.kbulk, &self.mu);
            let hg = &self.hg;
            let active = &self.active;
            let touched = &self.touched;
            self.bonds
                .par_chunks_mut(per)
                .zip(self.locals.par_iter_mut())
                .enumerate()
                .map(|(c, (bchunk, loc))| {
                    let base = c * per;
                    let mut tot = Tot::default();
                    let v2 = vol * vol;
                    for (ii, bonds) in bchunk.iter_mut().enumerate() {
                        let i = base + ii;
                        if !active[i] {
                            continue;
                        }
                        let me = &p[i];
                        let fi = fixed[i].load(Ordering::Relaxed);
                        let ai = awake[i];
                        let b = part[i] as usize;
                        let mat = &mats[b];
                        let cvv = c_bond[b] * v2;
                        let fail = mat.fail_stretch;
                        let sy0 = mat.yield_strength / mat.young * rate_gain;
                        let hard = mat.hardening / mat.young * rate_gain;
                        let comp = mat.comp_factor;
                        let rate_c = mat.rate_c;
                        // materiais frágeis têm bem mais dispersão de resistência
                        let nf = (noise * if comp >= 5.0 { 2.0 } else { 1.0 }).min(0.6);
                        let heat = TAYLOR_QUINNEY * 0.5 / (mat.rho * mat.cp * vol);
                        let (mut ax, mut ay, mut az, mut acc_t) = (0.0f64, 0.0f64, 0.0f64, 0.0f32 as f64);
                        let (mut acc_s, mut acc_w) = (0.0f32, 0.0f32);
                        for bd in bonds.iter_mut() {
                            // ligação rompida (ixi < 0 guarda −1/|ξ|): perdeu a tração, mas os
                            // fragmentos ainda se empurram quando comprimidos
                            let broken = bd.ixi < 0.0;
                            let j = bd.j as usize;
                            if fi && fixed[if j < nr { j } else { src[j - nr] as usize }].load(Ordering::Relaxed) {
                                continue;
                            }
                            if !ai && !awake[if j < nr { j } else { src[j - nr] as usize }] {
                                continue; // as duas pontas dormem: deformação nula
                            }
                            // destino da reação: nó real j, ou (fantasma) o real de origem com força espelhada
                            let (tgt, sy, sz, wgt) = if j < nr {
                                (j, -1.0, -1.0, 1.0)
                            } else {
                                let k = src[j - nr] as usize;
                                let m = gmask[j - nr];
                                (k, if m & 1 != 0 { 1.0 } else { -1.0 }, if m & 2 != 0 { 1.0 } else { -1.0 }, if k == i { 0.5 } else { 1.0 })
                            };
                            let q = &p[j];
                            let (rx, ry, rz) = (q.x - me.x, q.y - me.y, q.z - me.z);
                            let r = (rx * rx + ry * ry + rz * rz).sqrt().max(1e-12);
                            let ixi = (bd.ixi as f64).abs();
                            let s = r * ixi - 1.0;
                            if broken && osb {
                                let xi = 1.0 / ixi;
                                let e = r - xi;
                                if e < bd.sp as f64 {
                                    // parte volumétrica da ligação rompida (derivada de min(e, e_ruptura))
                                    let vj = vol * bd.w as f64;
                                    let bj = part[j] as usize;
                                    let t_i = 3.0 * kbulk[b] * theta[i] as f64 * xi / (mvol[i] as f64).max(1e-30) - thp[i] as f64 * xi * rsc[i] as f64;
                                    let t_j = 3.0 * kbulk[bj] * theta[tgt] as f64 * xi / (mvol[tgt] as f64).max(1e-30) - thp[tgt] as f64 * xi * rsc[tgt] as f64;
                                    let k = (t_i + t_j) * vol * vj / r;
                                    let (fx, fy, fz) = (k * rx, k * ry, k * rz);
                                    ax += fx;
                                    ay += fy;
                                    az += fz;
                                    if tgt != i {
                                        if !awake[tgt] {
                                            touched[tgt].store(true, Ordering::Relaxed);
                                        }
                                        let lt = &mut loc[tgt];
                                        lt[0] -= fx as f32;
                                        lt[1] += (sy * fy) as f32;
                                        lt[2] += (sz * fz) as f32;
                                    }
                                }
                            }
                            if broken {
                                // state-based: a mola só pega a compressão além da que havia na ruptura (sem salto)
                                let s_el = if osb { s - (bd.sp as f64 * ixi).min(0.0) } else { s - bd.sp as f64 };
                                if s_el >= 0.0 {
                                    continue;
                                }
                                // compressão do material fraturado: só elástica, sem escoamento
                                let w = bd.w as f64;
                                let k = cvv * s_el * w / r;
                                let (fx, fy, fz) = (k * rx, k * ry, k * rz);
                                ax += fx;
                                ay += fy;
                                az += fz;
                                if tgt != i {
                                    if !awake[tgt] {
                                        touched[tgt].store(true, Ordering::Relaxed);
                                    }
                                    let lt = &mut loc[tgt];
                                    lt[0] -= fx as f32;
                                    lt[1] += (sy * fy) as f32;
                                    lt[2] += (sz * fz) as f32;
                                }
                                tot.e_el += wgt * 0.5 * cvv * s_el * s_el * w / ixi;
                                continue;
                            }
                            // estiramento médio local (parte volumétrica) do passo anterior
                            let sb = if dev { 0.5 * (sbar[i] + sbar[tgt]) as f64 } else { 0.0 };
                            // somas para o estiramento médio do próximo passo
                            let wf = bd.w;
                            let s_elastic = (s - bd.sp as f64) as f32;
                            if dev {
                                acc_s += s_elastic * wf;
                                acc_w += wf;
                            }
                            let sp = bd.sp as f64;
                            let w = bd.w as f64;
                            let u = bd.rnd as f64 * (1.0 / 127.5) - 1.0;
                            if !osb && s > fail * (1.0 + nf * u) {
                                let s_el = s - sp;
                                tot.e_fr += wgt * 0.5 * cvv * s_el * s_el * w / ixi;
                                bd.ixi = -bd.ixi;
                                continue;
                            }
                            if osb {
                                // ---- state-based (LPS + plasticidade J2 de Mitchell), ω = 1
                                let xi = 1.0 / ixi;
                                let e = r - xi;
                                let vj = vol * w;
                                let bj = part[j] as usize;
                                let (th_i, m_i, rs_i) = (theta[i] as f64, (mvol[i] as f64).max(1e-30), rsc[i] as f64);
                                let (th_j, m_j, rs_j) = (theta[tgt] as f64, (mvol[tgt] as f64).max(1e-30), rsc[tgt] as f64);
                                let (a_i, a_j) = (15.0 * mu[b] / m_i, 15.0 * mu[bj] / m_j);
                                let (ed_i, ed_j) = (e - th_i * xi / 3.0, e - th_j * xi / 3.0);
                                let (dp_i, dp_j) = (bd.sp as f64, bd.acc as f64);
                                let (tr_i, tr_j) = (a_i * (ed_i - dp_i), a_j * (ed_j - dp_j));
                                // forças com o termo de acoplamento (derivada exata ⇒ conservativas)
                                let (fc_i, fc_j) = (rs_i * (tr_i - thp[i] as f64 * xi), rs_j * (tr_j - thp[tgt] as f64 * xi));
                                let (td_i, td_j) = (rs_i * tr_i, rs_j * tr_j);
                                // ruptura: estiramento crítico (trinca em tração/frágil) ou dano dúctil de Johnson-Cook
                                let dthr = 1.0 + 0.5 * nf * u;
                                let _ = (dthr, dmg);
                                if s > fail * (1.0 + nf * u) {
                                    // ruptura: a energia elástica desviadora da ligação é perdida
                                    tot.e_fr += wgt * (0.5 * td_i * td_i / a_i + 0.5 * td_j * td_j / a_j) * vol * vj;
                                    bd.ixi = -bd.ixi;
                                    // guarda a extensão na ruptura (limite da contribuição volumétrica)
                                    bd.sp = e as f32;
                                    bd.acc = 0.0;
                                    continue;
                                }
                                // retorno ao escoamento: a parte desviadora que passa do limite vira plástica
                                let (mut dw_i, mut dw_j) = (0.0, 0.0);
                                if rs_i < 1.0 {
                                    let nw = ed_i - td_i / a_i;
                                    dw_i = td_i * (nw - dp_i);
                                    bd.sp = nw as f32;
                                }
                                if rs_j < 1.0 {
                                    let nw = ed_j - td_j / a_j;
                                    dw_j = td_j * (nw - dp_j);
                                    bd.acc = nw as f32;
                                }
                                let t_i = 3.0 * kbulk[b] * th_i * xi / m_i + fc_i;
                                let t_j = 3.0 * kbulk[bj] * th_j * xi / m_j + fc_j;
                                let k = (t_i + t_j) * vol * vj / r;
                                let (fx, fy, fz) = (k * rx, k * ry, k * rz);
                                ax += fx;
                                ay += fy;
                                az += fz;
                                // somas do nó para o próximo passo: θ, m, norma² do desvio, trabalho plástico
                                let l = &mut loc[i];
                                l[3] += (dw_i * vj) as f32;
                                if tgt != i {
                                    if !awake[tgt] {
                                        touched[tgt].store(true, Ordering::Relaxed);
                                    }
                                    let lt = &mut loc[tgt];
                                    lt[0] -= fx as f32;
                                    lt[1] += (sy * fy) as f32;
                                    lt[2] += (sz * fz) as f32;
                                    lt[3] += (dw_j * vj) as f32;
                                }
                                tot.e_el += wgt * (0.5 * td_i * td_i / a_i + 0.5 * td_j * td_j / a_j) * vol * vj;
                                tot.e_pl += wgt * (dw_i + dw_j) * vol * vj;
                                continue;
                            }
                            let ea = bd.acc as f64;
                            // encruamento que satura (como Johnson-Cook, n ≈ 0,3): não dobra a resistência em grandes deformações
                            let mut sy_t = (sy0 + hard * ea / (1.0 + ea * INV_HSAT)) * (0.5 * (me.soft + q.soft)) * (1.0 + 0.3 * nf * u);
                            let mut s_el = s - sp;
                            let mut dtemp = 0.0;
                            // plastifica só a parte desviadora (s_el − sb); a volumétrica fica elástica
                            let sd = s_el - sb;
                            if (sd > sy_t || sd < -sy_t * comp) && rate_c > 0.0 {
                                // Johnson-Cook: resistência cresce com a taxa de estiramento da ligação
                                // (calculada só nas ligações que iam escoar, quase sem custo)
                                let rate = (((q.vx - me.vx) * rx + (q.vy - me.vy) * ry + (q.vz - me.vz) * rz) / r * ixi).abs();
                                if rate > 1.0 {
                                    sy_t *= 1.0 + rate_c * rate.ln();
                                }
                            }
                            if sd > sy_t || sd < -sy_t * comp {
                                let lim = if sd > 0.0 { sy_t } else { -sy_t * comp };
                                let dsp = sd - lim;
                                bd.sp = (sp + dsp) as f32;
                                bd.acc += dsp.abs() as f32;
                                // dissipação = tensão elástica APÓS o retorno ao limite × incremento plástico
                                // (no caso desviador, a tensão inclui a parte volumétrica: sb + lim)
                                s_el -= dsp;
                                let de = cvv * s_el.abs() * dsp.abs() * w / ixi;
                                tot.e_pl += wgt * de;
                                dtemp = heat * de;
                                acc_t += dtemp;
                            }
                            let f = cvv * s_el * w;
                            let k = f / r;
                            let (fx, fy, fz) = (k * rx, k * ry, k * rz);
                            ax += fx;
                            ay += fy;
                            az += fz;
                            if tgt != i {
                                if !awake[tgt] {
                                    touched[tgt].store(true, Ordering::Relaxed);
                                }
                                let lt = &mut loc[tgt];
                                if dev {
                                    lt[4] += s_elastic * wf;
                                    lt[5] += wf;
                                }
                                lt[0] -= (fx) as f32;
                                lt[1] += (sy * fy) as f32;
                                lt[2] += (sz * fz) as f32;
                                lt[3] += (dtemp) as f32;
                            }
                            tot.e_el += wgt * 0.5 * cvv * s_el * s_el * w / ixi;
                        }
                        loc[i][0] += (ax) as f32;
                        loc[i][1] += (ay) as f32;
                        loc[i][2] += (az) as f32;
                        loc[i][3] += (acc_t) as f32;
                        if dev {
                            loc[i][4] += acc_s;
                            loc[i][5] += acc_w;
                        }
                        if !ai && (ax != 0.0 || ay != 0.0 || az != 0.0) {
                            touched[i].store(true, Ordering::Relaxed);
                        }
                        // contato por penalidade. Só nós acordados varrem; um par (acordado, dormindo)
                        // é tratado pelo acordado, e (acordado, acordado) uma vez só.
                        if ai {
                            let c0 = hg.cell[i];
                            let offs: &[(i32, i32, i32)] = if all_awake && !near[i] { &FORWARD } else { &ALL27 };
                            for &(ox, oy, oz) in offs {
                                let tc = [c0[0] + ox, c0[1] + oy, c0[2] + oz];
                                let same = ox == 0 && oy == 0 && oz == 0;
                                let fwd = oz > 0 || (oz == 0 && (oy > 0 || (oy == 0 && ox > 0)));
                                let mut cur = hg.head[hg.bucket(tc)];
                                while cur != NIL {
                                    let j = cur as usize;
                                    cur = hg.next[j];
                                    if hg.cell[j] != tc || j == i || (fi && fixed[if j < nr { j } else { src[j - nr] as usize }].load(Ordering::Relaxed)) {
                                        continue;
                                    }
                                    let (org, is_ghost) = if j < nr { (j, false) } else { (src[j - nr] as usize, true) };
                                    if is_ghost && !near[i] {
                                        continue;
                                    }
                                    let aj = awake[org];
                                    if aj {
                                        // par entre dois acordados: uma vez só
                                        if is_ghost {
                                            if org < i {
                                                continue;
                                            }
                                        } else if !(fwd || (same && j > i)) {
                                            continue;
                                        }
                                    }
                                    let rc = if part[j] == part[i] { 0.8 * dx } else { 0.95 * dx };
                                    let (rx, ry, rz) = (p[j].x - me.x, p[j].y - me.y, p[j].z - me.z);
                                    let r2 = rx * rx + ry * ry + rz * rz;
                                    if r2 >= rc * rc {
                                        continue;
                                    }
                                    let r = r2.sqrt().max(1e-9);
                                    let s = (r - rc) / rc;
                                    let kk = c_ct * s * v2 / r;
                                    let (fx, fy, fz) = (kk * rx, kk * ry, kk * rz);
                                    loc[i][0] += fx as f32;
                                    loc[i][1] += fy as f32;
                                    loc[i][2] += fz as f32;
                                    let wgt = if !is_ghost {
                                        if !aj {
                                            touched[j].store(true, Ordering::Relaxed);
                                        }
                                        let lj = &mut loc[j];
                                        lj[0] -= fx as f32;
                                        lj[1] -= fy as f32;
                                        lj[2] -= fz as f32;
                                        1.0
                                    } else if org == i {
                                        0.5
                                    } else {
                                        let m = gmask[j - nr];
                                        let (sy, sz) = (if m & 1 != 0 { 1.0 } else { -1.0 }, if m & 2 != 0 { 1.0 } else { -1.0 });
                                        if !aj {
                                            touched[org].store(true, Ordering::Relaxed);
                                        }
                                        loc[org][0] -= fx as f32;
                                        loc[org][1] += (sy * fy) as f32;
                                        loc[org][2] += (sz * fz) as f32;
                                        1.0
                                    };
                                    tot.e_ct += wgt * 0.5 * c_ct * s * s * rc * v2;
                                }
                            }
                        }
                    }
                    tot
                })
                .collect()
        };
        let t2 = Instant::now();
        {
            // ponteiros dos buffers locais: cada índice i é acessado por uma única thread
            let ptrs: Vec<SyncPtr> = self.locals.iter_mut().map(|v| SyncPtr(v.as_mut_ptr())).collect();
            let part = &self.part;
            let fixed = &self.fixed;
            // state-based: as ondas de pressão chegam fortes à borda; a fixação solta só por dano
            // (ligações rompidas / dano dúctil), não pela força de uma onda passando
            let release_dv = if self.osb { f64::INFINITY } else { 0.02 * self.cfg.velocity };
            let mass = &self.mass;
            let mats = &self.mats;
            let touched = &self.touched;
            let sleep_v = self.sleep_v;
            let osb = self.osb;
            let (kbulk, mu) = (&self.kbulk, &self.mu);
            let rate_gain = self.cfg.rate_gain;
            let vol = self.vol;
            let osbv = SyncOsb(self.theta.as_mut_ptr(), self.mvol.as_mut_ptr(), self.rsc.as_mut_ptr(), self.tdn2.as_mut_ptr(), self.epq.as_mut_ptr(), self.epr.as_mut_ptr());
            let dmgp = SyncPtr1(self.dmg.as_mut_ptr());
            let dmgp = &dmgp;
            let (jcd, mcur) = (&self.jcd, &self.mcur);
            let osbv = &osbv;
            let e_vol: f64 = self.p[..nr].par_iter_mut().zip(self.awake.par_iter_mut()).zip(self.sbar.par_iter_mut()).enumerate().with_min_len(256).map(|(i, ((q, awake), sbar_i))| -> f64 {
                let was_touched = touched[i].swap(false, Ordering::Relaxed);
                // nó que acorda agora teve só parte das ligações somadas: mantém o estado anterior
                let was_awake = *awake;
                if !*awake && !was_touched {
                    // dormindo e sem força: os buffers dele já estão zerados
                    return if osb { unsafe { 0.5 * kbulk[part[i] as usize] * (*osbv.0.add(i) as f64).powi(2) * vol } } else { 0.0 };
                }
                let mut f = [0.0f64; 8];
                for pt in &ptrs {
                    // SAFETY: índice i exclusivo desta thread; o buffer não é usado em outro lugar agora
                    let v = unsafe { &mut *pt.0.add(i) };
                    for k in 0..8 {
                        f[k] += v[k] as f64;
                    }
                    *v = [0.0; 8];
                }
                if !osb && f[5] > 0.0 {
                    *sbar_i = (f[4] / f[5]) as f32;
                }
                if !*awake {
                    // dormindo: acorda quando a força o aceleraria ao limiar de velocidade em ~10 passos
                    let mm = mass[part[i] as usize];
                    let fthr = sleep_v * mm / (10.0 * dt);
                    if f[0] * f[0] + f[1] * f[1] + f[2] * f[2] <= fthr * fthr {
                        return if osb { unsafe { 0.5 * kbulk[part[i] as usize] * (*osbv.0.add(i) as f64).powi(2) * vol } } else { 0.0 };
                    }
                    *awake = true;
                }
                let b = part[i] as usize;
                let m = &mats[b];
                let mut e_v = 0.0;
                if osb && was_awake {
                    // SAFETY: índice i exclusivo desta thread
                    unsafe {
                        let th = *osbv.0.add(i) as f64;
                        // trabalho plástico (por volume) → deformação plástica equivalente e calor
                        let ep_old = *osbv.4.add(i) as f64;
                        let rate = *osbv.5.add(i) as f64;
                        let jc = if m.rate_c > 0.0 && rate > 1.0 { 1.0 + m.rate_c * rate.ln() } else { 1.0 };
                        let sy = (m.yield_strength + m.hardening * ep_old / (1.0 + ep_old * INV_HSAT)) * rate_gain * q.soft.max(0.03) * jc;
                        let dw = f[3].max(0.0);
                        let dep = dw / sy.max(1.0);
                        *osbv.4.add(i) = (ep_old + dep) as f32;
                        *osbv.5.add(i) = (dep / dt) as f32;
                        q.temp += TAYLOR_QUINNEY * dw / (m.rho * m.cp);
                        e_v = 0.5 * kbulk[b] * th * th * vol;
                        // dano dúctil: Δεp / εf(triaxialidade)
                        if let (Some((d1, d2, d3)), true) = (jcd[b], dep > 0.0) {
                            let svm = (mcur[i] as f64 * *osbv.3.add(i) as f64 / 5.0).max(0.0).sqrt().max(1e3);
                            let tri = (kbulk[b] * th / svm).clamp(-3.0, 3.0);
                            let ef = (d1 + d2 * (d3 * tri).exp()).clamp(0.008, 10.0);
                            *dmgp.0.add(i) += (dep / ef) as f32;
                        }
                        let _ = mu;
                    }
                } else if !osb {
                    q.temp += f[3];
                } else {
                    e_v = unsafe { 0.5 * kbulk[b] * (*osbv.0.add(i) as f64).powi(2) * vol };
                }
                let theta = ((q.temp - T_REF) / (m.t_melt - T_REF)).clamp(0.0, 1.0);
                q.soft = (1.0 - theta).max(0.03);
                let mm = mass[part[i] as usize];
                if fixed[i].load(Ordering::Relaxed) {
                    // a fixação só segura cargas moderadas (ondas, flexão); impacto direto a arranca
                    let dv = (f[0] * f[0] + f[1] * f[1] + f[2] * f[2]).sqrt() / mm * dt;
                    if dv > release_dv {
                        fixed[i].store(false, Ordering::Relaxed);
                    } else {
                        q.vx = 0.0;
                        q.vy = 0.0;
                        q.vz = 0.0;
                        return e_v;
                    }
                }
                q.vx += f[0] / mm * dt;
                q.vy += f[1] / mm * dt;
                q.vz += f[2] / mm * dt;
                q.x += q.vx * dt;
                q.y += q.vy * dt;
                q.z += q.vz * dt;
                e_v
            }).sum();
            self.cur_e_vol = e_vol;
        }
        // nós que acordaram agora: entram na grade dinâmica e acordam a vizinhança de ligações
        self.wake_events();
        let (mut e_el, mut e_ct) = (0.0, 0.0);
        for t in &tots {
            self.e_pl += t.e_pl;
            self.e_fr += t.e_fr;
            e_el += t.e_el;
            e_ct += t.e_ct;
        }
        self.cur_e_el = e_el + self.cur_e_vol;
        self.cur_e_ct = e_ct;
        let t3 = Instant::now();
        self.prof[0] += (t1 - t0).as_secs_f64();
        self.prof[1] += (t2 - t1).as_secs_f64();
        self.prof[2] += (t3 - t2).as_secs_f64();
        self.time += dt;
        self.step_count += 1;
    }

    /// Fim por separação: depois de ter havido contato, quando o corpo principal do projétil
    /// (nós com ≥ 50% das ligações vivas, sem fragmentos soltos) deixa de tocar o alvo, a
    /// animação continua por mais 25% do tempo decorrido até esse instante.
    fn check_exit(&mut self) {
        let nr = self.n_real;
        let mut alive = vec![0u32; nr];
        for i in 0..nr {
            for bd in &self.bonds[i] {
                if bd.ixi < 0.0 {
                    continue;
                }
                alive[i] += 1;
                let j = bd.j as usize;
                let t = if j < nr { j } else { self.src[j - nr] as usize };
                if t != i {
                    alive[t] += 1;
                }
            }
        }
        // fixação danificada (perdeu ligações para o material que rasgou) solta o nó
        for i in 0..nr {
            let ductile = self.osb && self.dmg[i] > 0.5;
            if (alive[i] < (0.75 * self.initial_bonds[i] as f64) as u32 || ductile) && self.fixed[i].load(Ordering::Relaxed) {
                self.fixed[i].store(false, Ordering::Relaxed);
            }
        }
        let reach2 = (1.3 * self.cfg.dx).powi(2);
        let (mut touching, mut n_main, mut x_main) = (false, 0usize, 0.0f64);
        // corpo principal = nós com ≥ 50% das ligações vivas; se o projétil se estilhaçou e não
        // sobrou nenhum, usa todos os nós do projétil
        let strict = |i: usize| alive[i] as f64 >= 0.5 * self.initial_bonds[i] as f64;
        let any_main = (0..nr).any(|i| self.part[i] < self.n_proj && strict(i));
        let is_main = |i: usize| self.part[i] < self.n_proj && (!any_main || strict(i));
        let (mut x_min, mut x_max, mut vx_main) = (f64::MAX, f64::MIN, 0.0f64);
        let (mut vy_main, mut vz_main) = (0.0f64, 0.0f64);
        for i in 0..nr {
            if is_main(i) {
                x_main += self.p[i].x;
                vx_main += self.p[i].vx;
                vy_main += self.p[i].vy;
                vz_main += self.p[i].vz;
                n_main += 1;
                x_min = x_min.min(self.p[i].x);
                x_max = x_max.max(self.p[i].x);
            }
        }
        'nodes: for i in 0..nr {
            if !is_main(i) {
                continue;
            }
            let c0 = self.hg.cell[i];
            let me = &self.p[i];
            for oz in -1..=1 {
                for oy in -1..=1 {
                    for ox in -1..=1 {
                        let tc = [c0[0] + ox, c0[1] + oy, c0[2] + oz];
                        let mut cur = self.hg.head[self.hg.bucket(tc)];
                        while cur != NIL {
                            let j = cur as usize;
                            cur = self.hg.next[j];
                            if self.hg.cell[j] != tc || self.part[j] < self.n_proj {
                                continue;
                            }
                            // só conta o alvo de verdade: material inteiro ainda dentro da chapa
                            // (fragmentos e tampão que viajam com o projétil não o prendem)
                            let jr = if j < nr { j } else { self.src[j - nr] as usize };
                            let q = &self.p[j];
                            if (alive[jr] as f64) < 0.5 * self.initial_bonds[jr] as f64 || q.x > self.back_x + self.cfg.dx {
                                continue;
                            }
                            let d2 = (q.x - me.x).powi(2) + (q.y - me.y).powi(2) + (q.z - me.z).powi(2);
                            if d2 < reach2 {
                                touching = true;
                                break 'nodes;
                            }
                        }
                    }
                }
            }
        }
        // o corpo só "saiu" se o centro de massa estiver fora do alvo (depois da face traseira ou
        // antes da frontal); a cauda de um dardo dentro do túnel já aberto não toca as paredes.
        // Se estiver a mais de um comprimento de projétil, vale como separado mesmo que um tampão
        // ejetado ainda viaje colado a ele.
        let xcm = if n_main > 0 { x_main / n_main as f64 } else { 0.0 };
        let vxm = if n_main > 0 { vx_main / n_main as f64 } else { 0.0 };
        // saída pela frente só conta se ele estiver se afastando (senão é só a aproximação inicial)
        // ricochete: ainda na frente do alvo e sem contato há um tempo (a 80° vx pode continuar > 0)
        let grace = 0.25 * self.cfg.design.projectile.length() / self.cfg.velocity.max(1.0);
        let bounced = xcm < 0.0 && (vxm < 0.0 || self.time - self.last_touch > grace);
        // ricochete raspando: sem contato há um tempo e andando quase paralelo à placa (ou se
        // afastando). Distingue da cauda de um dardo no túnel, que segue entrando (vx alto).
        let nmf = n_main.max(1) as f64;
        let speed = ((vx_main / nmf).powi(2) + (vy_main / nmf).powi(2) + (vz_main / nmf).powi(2)).sqrt();
        let skim = !touching && self.had_contact && self.time - self.last_touch > grace && vxm < 0.3 * speed;
        let outside = xcm > self.back_x || bounced || skim;
        // corpo inteiro fora do alvo (nó mais recuado depois da face traseira, ou o mais avançado antes
        // da frontal): o que ainda encosta nele é o tampão ejetado, não o alvo
        let len = self.cfg.design.projectile.length();
        let far_out = n_main > 0 && (x_min > self.back_x || (x_max < 0.0 && vxm < 0.0));
        let _ = len;
        if std::env::var("PENSIM_DBG").is_ok() && self.step_count % 200 == 0 {
            eprintln!("t={:.0}µs contato_antes={} tocando={} n_main={} xcm={:.1}mm back={:.1}mm outside={} fora_total={}", self.time * 1e6, self.had_contact, touching, n_main, xcm * 1e3, self.back_x * 1e3, outside, far_out);
        }
        if touching {
            self.had_contact = true;
            self.last_touch = self.time;
        }
        // separação: só para classificar o resultado (não encerra a simulação)
        if !self.exit_found && self.had_contact && n_main > 0 && outside && (!touching || far_out) {
            self.exit_found = true;
            self.sep_time = Some(self.time);
        }
        // ÚNICO critério de fim: a traseira do corpo principal do projétil (só o que continua
        // preso a ele; fragmentos não contam) andou o comprimento do projétil + 5× a espessura do alvo
        if n_main > 0 {
            let d = self.dir;
            // deslocamento médio dos nós do corpo principal em relação à posição inicial de cada um
            // (a traseira de um corpo inteiro anda o mesmo que o centro dele)
            let mut acc = 0.0;
            for i in 0..nr {
                if is_main(i) {
                    let p0 = self.pos0[i];
                    acc += (self.p[i].x - p0[0]) * d[0] + (self.p[i].z - p0[2]) * d[2];
                }
            }
            self.travel = (acc / n_main as f64).max(0.0);
            if self.travel >= self.travel_goal {
                self.t_end = self.t_end.min(self.time);
            }
        }
    }

    pub fn advance_for(&mut self, budget: Duration) {
        let start = Instant::now();
        while !self.done() {
            if self.time >= self.next_frame_t {
                self.capture();
                self.captured += 1;
                if self.captured >= self.cfg.n_frames {
                    self.frame_dt *= 2.0; // evento mais longo que o previsto: espaça mais
                    self.captured = self.cfg.n_frames / 2;
                }
                self.next_frame_t += self.frame_dt;
            }
            for _ in 0..2 {
                self.step();
            }
            if self.step_count % 10 == 0 {
                self.check_exit();
            }
            if start.elapsed() >= budget {
                break;
            }
        }
        if self.done() && self.frames.last().map_or(true, |f| f.time < self.time - 1e-12) {
            self.capture();
        }
    }

    pub fn run_to_end(&mut self) {
        while !self.done() {
            self.advance_for(Duration::from_secs(3600));
        }
    }

    /// Encerra agora (o usuário decidiu parar): grava o quadro atual e fixa o fim neste instante.
    pub fn finish_now(&mut self) {
        self.capture();
        self.t_end = self.time;
    }

    pub fn capture(&mut self) {
        self.sync_ghosts();
        let n = self.n_real;
        let vol = self.vol;
        // tensões por virial: cada ligação (guardada uma vez) contribui aos dois nós
        let nch = self.locals.len().max(1);
        let per = (n + nch - 1) / nch;
        let parts: Vec<Vec<[f64; 8]>> = self
            .bonds
            .par_chunks(per)
            .enumerate()
            .map(|(c, bchunk)| {
                let mut loc = vec![[0.0f64; 8]; n]; // xx yy zz xy yz zx, ε plástica, ligações vivas
                for (ii, bonds) in bchunk.iter().enumerate() {
                    let i = c * per + ii;
                    let me = &self.p[i];
                    let mat = &self.mats[self.part[i] as usize];
                    let cvv = self.c_bond[self.part[i] as usize] * vol * vol;
                    let sy0 = mat.yield_strength / mat.young;
                    let hard = mat.hardening / mat.young;
                    for bd in bonds {
                        let j = bd.j as usize;
                        // parceiro real (ou origem do fantasma) e máscara de espelhamento do tensor
                        let (t, sy, sz) = if j < n {
                            (j, 1.0, 1.0)
                        } else {
                            let m = self.gmask[j - n];
                            (self.src[j - n] as usize, if m & 1 != 0 { -1.0 } else { 1.0 }, if m & 2 != 0 { -1.0 } else { 1.0 })
                        };
                        loc[i][6] += bd.acc as f64;
                        if t != i {
                            loc[t][6] += bd.acc as f64;
                        }
                        if bd.ixi < 0.0 {
                            continue;
                        }
                        loc[i][7] += 1.0;
                        if t != i {
                            loc[t][7] += 1.0;
                        }
                        let q = &self.p[j];
                        let (rx, ry, rz) = (q.x - me.x, q.y - me.y, q.z - me.z);
                        let r = (rx * rx + ry * ry + rz * rz).sqrt().max(1e-12);
                        let s = r * bd.ixi as f64 - 1.0;
                        let ea = bd.acc as f64;
                        let sy_t = (sy0 + hard * ea / (1.0 + ea * INV_HSAT)) * (0.5 * (me.soft + q.soft));
                        let sb = if self.cfg.deviatoric_plasticity { 0.5 * (self.sbar[i] + self.sbar[t]) as f64 } else { 0.0 };
                        let s_el = sb + (s - bd.sp as f64 - sb).clamp(-sy_t * mat.comp_factor, sy_t);
                        let f = cvv * s_el * bd.w as f64;
                        let k = 0.5 * f / r / vol;
                        let v = [k * rx * rx, k * ry * ry, k * rz * rz, k * rx * ry, k * ry * rz, k * rz * rx];
                        for m in 0..6 {
                            loc[i][m] += v[m];
                        }
                        if t != i {
                            // tensor espelhado: xy troca com sy, yz com sy·sz, zx com sz
                            let sgn = [1.0, 1.0, 1.0, sy, sy * sz, sz];
                            for m in 0..6 {
                                loc[t][m] += sgn[m] * v[m];
                            }
                        }
                    }
                }
                loc
            })
            .collect();
        let tens: Vec<([f32; 6], f32, f32)> = (0..n)
            .into_par_iter()
            .map(|i| {
                let mut a = [0.0f64; 8];
                for l in &parts {
                    for m in 0..8 {
                        a[m] += l[i][m];
                    }
                }
                let ib = self.initial_bonds[i].max(1) as f64;
                (
                    [a[0] as f32, a[1] as f32, a[2] as f32, a[3] as f32, a[4] as f32, a[5] as f32],
                    (a[6] / ib) as f32,
                    (1.0 - a[7] / ib) as f32,
                )
            })
            .collect();
        let von: Vec<f32> = tens
            .iter()
            .map(|(s, _, _)| {
                (0.5 * ((s[0] - s[1]).powi(2) + (s[1] - s[2]).powi(2) + (s[2] - s[0]).powi(2))
                    + 3.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5]))
                    .max(0.0)
                    .sqrt()
            })
            .collect();
        let mut press: Vec<f32> = tens.iter().map(|(s, _, _)| -(s[0] + s[1] + s[2]) / 3.0).collect();
        let mut von = von;
        let mut plastic: Vec<f32> = tens.iter().map(|t| t.1).collect();
        if self.osb {
            // state-based: pressão = −Kθ; von Mises da norma do desvio (σ_vm² = m·‖t_d‖²/5)
            for i in 0..n {
                let b = self.part[i] as usize;
                press[i] = (-self.kbulk[b] * self.theta[i] as f64) as f32;
                von[i] = ((self.mvol[i] as f64 * self.tdn2[i] as f64 / 5.0).max(0.0).sqrt()) as f32;
                plastic[i] = self.epq[i];
            }
        }
        let (mut ke_p, mut vx_p, mut np) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..n {
            if self.part[i] < self.n_proj {
                let q = self.p[i];
                ke_p += 0.5 * self.mass[self.part[i] as usize] * (q.vx * q.vx + q.vy * q.vy + q.vz * q.vz);
                np += 1.0;
            }
        }
        vx_p = self.residual_velocity();
        // parada antecipada: energia cinética do projétil estável por 4 quadros
        self.ke_hist.push(ke_p);
        let h = &self.ke_hist;
        if self.time > 0.3 * self.t_end && h.len() >= 5 {
            let w = &h[h.len() - 5..];
            let (mn, mx) = (w.iter().cloned().fold(f64::MAX, f64::min), w.iter().cloned().fold(f64::MIN, f64::max));
            // parada antecipada só se o projétil estiver de fato parado (energia desprezível e estável);
            // enquanto ele ainda avança dentro do alvo, a simulação continua até ele sair (ou até o tempo máximo)
            let _ = (mn, mx);
        }
        let (e_el, e_ct) = (self.cur_e_el, self.cur_e_ct);
        let frame = Frame {
            time: self.time,
            pos: self.p[..n].iter().map(|q| [q.x as f32, q.y as f32, q.z as f32]).collect(),
            speed: self.p[..n].iter().map(|q| (q.vx * q.vx + q.vy * q.vy + q.vz * q.vz).sqrt() as f32).collect(),
            von_mises: von,
            pressure: press,
            plastic,
            temp: self.p[..n].iter().map(|q| q.temp as f32).collect(),
            damage: tens.iter().map(|t| t.2).collect(),
            ke_proj: ke_p * self.gfac,
            ke_target: self.kinetic(Some(false)) * self.gfac,
            vx_proj: vx_p,
            e_elastic: e_el * self.gfac,
            e_contact: e_ct * self.gfac,
            e_plastic: self.e_pl * self.gfac,
            e_fracture: self.e_fr * self.gfac,
        };
        self.frames.push(crate::store::PackedFrame::pack(&frame, self.compress_frames));
    }

    /// Velocidade do projétil ao longo da trajetória: da parte que já saiu pela face traseira
    /// (se for > 5% da massa), senão média ponderada pela massa de todo o projétil.
    pub fn residual_velocity(&self) -> f64 {
        let d = self.dir;
        let (mut mp, mut mb, mut pv, mut pvb) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for i in 0..self.n_real {
            if self.part[i] < self.n_proj {
                let q = self.p[i];
                let m = self.mass[self.part[i] as usize];
                let v = q.vx * d[0] + q.vz * d[2];
                mp += m;
                pv += m * v;
                if q.x > self.back_x {
                    mb += m;
                    pvb += m * v;
                }
            }
        }
        if mb > 0.05 * mp { pvb / mb } else { pv / mp.max(1e-30) }
    }

    pub fn outcome(&self) -> Outcome {
        let n = self.n_real;
        let d = self.dir;
        // massas: o projétil tem peças de densidades diferentes (chumbo, aço, tungstênio…)
        let (mut mp, mut mb, mut pv, mut pvb, mut tip) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, f64::MIN);
        let (mut np, mut behind) = (0.0f64, 0.0f64);
        let (mut nt, mut ejected) = (0.0f64, 0.0f64);
        for i in 0..n {
            let q = self.p[i];
            if self.part[i] < self.n_proj {
                let m = self.mass[self.part[i] as usize];
                let v = q.vx * d[0] + q.vz * d[2]; // velocidade ao longo da trajetória original
                np += 1.0;
                mp += m;
                pv += m * v;
                tip = tip.max(q.x);
                if q.x > self.back_x {
                    behind += 1.0;
                    mb += m;
                    pvb += m * v;
                }
            } else {
                nt += 1.0;
                if q.x > self.back_x + 2.0 * self.cfg.dx {
                    ejected += 1.0;
                }
            }
        }
        // velocidade residual = o que saiu pela face traseira (fragmentos erodidos que ficaram presos
        // no furo, parados, não puxam a média para baixo); se nada saiu, a média do projétil todo
        let _ = (mb, pvb, pv);
        let v_res = self.residual_velocity();
        let behind_frac = behind / np.max(1.0);
        // perfurou: a maior parte passou, ou o que restou do corpo saiu pela face traseira
        // (dardos longos se erodem: o que atravessa pode ser bem menos que a metade)
        let exited = self.sep_time.is_some() && tip > self.back_x + 2.0 * self.cfg.dx && behind_frac > 0.1;
        let kind = if (behind_frac > 0.5 || exited) && v_res > 0.0 {
            OutcomeKind::Perforation
        } else if behind_frac < 0.5 && (self.sep_time.is_some() || (v_res < 0.0 && behind_frac < 0.05)) {
            // soltou-se do alvo sem atravessar: ricocheteou ou rebateu
            OutcomeKind::Rejected
        } else {
            // terminou preso dentro do alvo
            OutcomeKind::PartialPenetration
        };
        let ke_p = self.kinetic(Some(true));
        let ke0 = self.ke0.max(1e-12);
        let (e_el, e_ct) = (self.cur_e_el, self.cur_e_ct);
        let total = self.kinetic(None) + e_el + e_ct + self.e_pl + self.e_fr;
        Outcome {
            kind,
            v_residual: v_res,
            penetration: (tip - 0.0).max(0.0),
            energy_absorbed_pct: 100.0 * (1.0 - ke_p / ke0),
            target_ejected_pct: 100.0 * ejected / nt.max(1.0),
            proj_behind_pct: 100.0 * behind_frac,
            energy_error_pct: 100.0 * (total - ke0) / ke0,
        }
    }
}
