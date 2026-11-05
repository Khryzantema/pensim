use crate::config::Config;
use crate::geometry::Model;
use crate::material::{Material, T_REF};
use rayon::prelude::*;
use std::time::{Duration, Instant};

const TAYLOR_QUINNEY: f64 = 0.9;
/// fração do limite de estabilidade do passo explícito
const DT_SAFETY: f64 = 0.65;

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
    pub frames: Vec<Frame>,
    p: Vec<P>,
    /// buffers locais (fx, fy, fz, dT) por bloco de nós: forças acumuladas sem corridas
    locals: Vec<Vec<[f64; 4]>>,
    cur_e_el: f64,
    cur_e_ct: f64,
    pos0: Vec<[f64; 3]>,
    part: Vec<u8>,
    fixed: Vec<bool>,
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
    exit_found: bool,
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

/// Deslocamentos de células "à frente": a própria célula + 13 vizinhas (meia vizinhança).
const FORWARD: [(i64, i64, i64); 14] = [
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
        if let Some(av) = crate::config::available_memory_mb() {
            let need = cfg.estimate_memory_mb();
            if need > 0.85 * av {
                return Err(format!(
                    "Memória insuficiente: esta malha precisa de ≈{:.0} MB e há ≈{:.0} MB livres. Aumente o espaçamento (aba RESOLUÇÃO) ou reduza os quadros gravados.",
                    need, av
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
            cur_e_ct: 0.0,
            p,
            pos0: model.pos,
            part: model.part,
            fixed: model.fixed,
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
            normals: vec![[0.0; 3]; n_real],
            sym,
            src,
            gmask,
            near,
            gfac,
            prof: [0.0; 3],
            exit_found: false,
            cfg,
        };
        sim.build_bonds(horizon);
        // blocos de trabalho: 4× as threads (melhor balanceamento, inclusive em núcleos heterogêneos),
        // limitados pela memória dos buffers locais (n_real × 32 B por bloco)
        let threads = rayon::current_num_threads().max(1);
        let mem_cap = (48_000_000 / (n_real.max(1) * 32)).max(threads);
        let nch = (threads * 4).min(mem_cap).max(1);
        sim.locals = (0..nch).map(|_| vec![[0.0; 4]; n_real]).collect();
        sim.dt = sim.stable_dt();
        sim.ke0 = sim.kinetic(Some(true));
        Ok(sim)
    }

    /// nós calculados (os quadros têm este tamanho)
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
                                        list.push(Bond { j: j as u32, ixi: (1.0 / xi) as f32, w: w as f32, sp: 0.0, acc: 0.0 });
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
        // nós de superfície: menos ligações que o máximo da peça
        let nparts = self.mats.len();
        let mut maxb = vec![0u32; nparts];
        for i in 0..nr {
            let p = self.part[i] as usize;
            maxb[p] = maxb[p].max(self.initial_bonds[i]);
        }
        for i in 0..nr {
            self.surface[i] = (self.initial_bonds[i] as f64) < 0.97 * maxb[self.part[i] as usize] as f64;
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
        self.sync_ghosts();
        self.build_grid(false, dx);
        let g = self.grid;
        let t1 = Instant::now();
        let nr = self.n_real;
        let nch = self.locals.len();
        let per = (nr + nch - 1) / nch;
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
            let cell_start = &self.cell_start;
            let cell_items = &self.cell_items;
            let cell_of = &self.cell_of;
            self.bonds
                .par_chunks_mut(per)
                .zip(self.locals.par_iter_mut())
                .enumerate()
                .map(|(c, (bchunk, loc))| {
                    for v in loc.iter_mut() {
                        *v = [0.0; 4];
                    }
                    let base = c * per;
                    let mut tot = Tot::default();
                    let v2 = vol * vol;
                    for (ii, bonds) in bchunk.iter_mut().enumerate() {
                        let i = base + ii;
                        let me = &p[i];
                        let fi = fixed[i];
                        let b = part[i] as usize;
                        let mat = &mats[b];
                        let cvv = c_bond[b] * v2;
                        let fail = mat.fail_stretch;
                        let sy0 = mat.yield_strength / mat.young;
                        let hard = mat.hardening / mat.young;
                        let comp = mat.comp_factor;
                        let heat = TAYLOR_QUINNEY * 0.5 / (mat.rho * mat.cp * vol);
                        let (mut ax, mut ay, mut az, mut acc_t) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
                        for bd in bonds.iter_mut() {
                            if bd.ixi < 0.0 {
                                continue;
                            }
                            let j = bd.j as usize;
                            if fi && fixed[j] {
                                continue;
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
                            let ixi = bd.ixi as f64;
                            let s = r * ixi - 1.0;
                            let sp = bd.sp as f64;
                            let w = bd.w as f64;
                            if s > fail {
                                let s_el = s - sp;
                                tot.e_fr += wgt * 0.5 * cvv * s_el * s_el * w / ixi;
                                bd.ixi = -1.0;
                                continue;
                            }
                            let sy_t = (sy0 + hard * bd.acc as f64) * (0.5 * (me.soft + q.soft));
                            let mut s_el = s - sp;
                            let mut dtemp = 0.0;
                            if s_el > sy_t || s_el < -sy_t * comp {
                                let lim = if s_el > 0.0 { sy_t } else { -sy_t * comp };
                                let dsp = s_el - lim;
                                bd.sp = (sp + dsp) as f32;
                                bd.acc += dsp.abs() as f32;
                                s_el = lim;
                                let de = cvv * lim.abs() * dsp.abs() * w / ixi;
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
                                let lt = &mut loc[tgt];
                                lt[0] -= fx;
                                lt[1] += sy * fy;
                                lt[2] += sz * fz;
                                lt[3] += dtemp;
                            }
                            tot.e_el += wgt * 0.5 * cvv * s_el * s_el * w / ixi;
                        }
                        loc[i][0] += ax;
                        loc[i][1] += ay;
                        loc[i][2] += az;
                        loc[i][3] += acc_t;
                        // contato por penalidade entre nós reais: cada par uma vez, só células "à frente"
                        let c0 = cell_coords(cell_of[i], &g);
                        for &(ox, oy, oz) in FORWARD.iter() {
                            let qc = [c0[0] + ox, c0[1] + oy, c0[2] + oz];
                            if (0..3).any(|k| qc[k] < 0 || qc[k] >= g.n[k]) {
                                continue;
                            }
                            let same_cell = ox == 0 && oy == 0 && oz == 0;
                            let id = ((qc[2] * g.n[1] + qc[1]) * g.n[0] + qc[0]) as usize;
                            for k in cell_start[id]..cell_start[id + 1] {
                                let j = cell_items[k as usize] as usize;
                                if j >= nr || (same_cell && j <= i) || (fi && fixed[j]) {
                                    continue;
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
                                loc[i][0] += fx;
                                loc[i][1] += fy;
                                loc[i][2] += fz;
                                loc[j][0] -= fx;
                                loc[j][1] -= fy;
                                loc[j][2] -= fz;
                                tot.e_ct += 0.5 * c_ct * s * s * rc * v2;
                            }
                        }
                        // contato com fantasmas (só nós perto de um plano de simetria): 27 células
                        if near[i] && !src.is_empty() {
                            for oz in -1..=1 {
                                for oy in -1..=1 {
                                    for ox in -1..=1 {
                                        let qc = [c0[0] + ox, c0[1] + oy, c0[2] + oz];
                                        if (0..3).any(|k| qc[k] < 0 || qc[k] >= g.n[k]) {
                                            continue;
                                        }
                                        let id = ((qc[2] * g.n[1] + qc[1]) * g.n[0] + qc[0]) as usize;
                                        for k in cell_start[id]..cell_start[id + 1] {
                                            let j = cell_items[k as usize] as usize;
                                            if j < nr || (fi && fixed[j]) {
                                                continue;
                                            }
                                            let org = src[j - nr] as usize;
                                            if org < i {
                                                continue; // o par espelhado é tratado a partir de `org`
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
                                            loc[i][0] += fx;
                                            loc[i][1] += fy;
                                            loc[i][2] += fz;
                                            let wgt = if org == i { 0.5 } else {
                                                let m = gmask[j - nr];
                                                let (sy, sz) = (if m & 1 != 0 { 1.0 } else { -1.0 }, if m & 2 != 0 { 1.0 } else { -1.0 });
                                                loc[org][0] -= fx;
                                                loc[org][1] += sy * fy;
                                                loc[org][2] += sz * fz;
                                                1.0
                                            };
                                            tot.e_ct += wgt * 0.5 * c_ct * s * s * rc * v2;
                                        }
                                    }
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
            let locals = &self.locals;
            let part = &self.part;
            let fixed = &self.fixed;
            let mass = &self.mass;
            let mats = &self.mats;
            self.p[..nr].par_iter_mut().enumerate().with_min_len(256).for_each(|(i, q)| {
                let mut f = [0.0f64; 4];
                for l in locals {
                    let v = &l[i];
                    f[0] += v[0];
                    f[1] += v[1];
                    f[2] += v[2];
                    f[3] += v[3];
                }
                q.temp += f[3];
                let m = &mats[part[i] as usize];
                let theta = ((q.temp - T_REF) / (m.t_melt - T_REF)).clamp(0.0, 1.0);
                q.soft = (1.0 - theta).max(0.03);
                if fixed[i] {
                    q.vx = 0.0;
                    q.vy = 0.0;
                    q.vz = 0.0;
                    return;
                }
                let mm = mass[part[i] as usize];
                q.vx += f[0] / mm * dt;
                q.vy += f[1] / mm * dt;
                q.vz += f[2] / mm * dt;
                q.x += q.vx * dt;
                q.y += q.vy * dt;
                q.z += q.vz * dt;
            });
        }
        let (mut e_el, mut e_ct) = (0.0, 0.0);
        for t in &tots {
            self.e_pl += t.e_pl;
            self.e_fr += t.e_fr;
            e_el += t.e_el;
            e_ct += t.e_ct;
        }
        self.cur_e_el = e_el;
        self.cur_e_ct = e_ct;
        let t3 = Instant::now();
        self.prof[0] += (t1 - t0).as_secs_f64();
        self.prof[1] += (t2 - t1).as_secs_f64();
        self.prof[2] += (t3 - t2).as_secs_f64();
        self.time += dt;
        self.step_count += 1;
    }

    /// Encerra pouco depois de o projétil sair do alvo (por trás, ao perfurar,
    /// ou pela frente, ao ricochetear), sem calcular até o tempo final.
    fn check_exit(&mut self) {
        if self.exit_found {
            return;
        }
        let margin = 0.005f64.max(3.0 * self.cfg.dx);
        let (mut np, mut behind, mut front, mut vx) = (0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for (i, q) in self.p[..self.n_real].iter().enumerate() {
            if self.part[i] >= self.n_proj {
                continue;
            }
            np += 1.0;
            vx += q.vx;
            if q.x > self.back_x + margin {
                behind += 1.0;
            }
            if q.x < -margin {
                front += 1.0;
            }
        }
        let vm = vx / np.max(1.0);
        let perforated = behind >= 0.5 * np && vm > 0.0;
        let rebounded = front >= np && vm < 0.0;
        if perforated || rebounded {
            self.exit_found = true;
            let tail = (0.2 * self.time).max(5e-6);
            self.t_end = self.t_end.min(self.time + tail);
        }
    }

    pub fn advance_for(&mut self, budget: Duration) {
        let start = Instant::now();
        while !self.done() {
            if self.time >= self.next_frame_t {
                self.capture();
                self.next_frame_t += self.t_end / self.cfg.n_frames.max(1) as f64;
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
                        let sy_t = (sy0 + hard * bd.acc as f64) * (0.5 * (me.soft + q.soft));
                        let s_el = (s - bd.sp as f64).clamp(-sy_t * mat.comp_factor, sy_t);
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
        let press: Vec<f32> = tens.iter().map(|(s, _, _)| -(s[0] + s[1] + s[2]) / 3.0).collect();
        let (mut ke_p, mut vx_p, mut np) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..n {
            if self.part[i] < self.n_proj {
                let q = self.p[i];
                ke_p += 0.5 * self.mass[self.part[i] as usize] * (q.vx * q.vx + q.vy * q.vy + q.vz * q.vz);
                vx_p += q.vx;
                np += 1.0;
            }
        }
        // parada antecipada: energia cinética do projétil estável por 4 quadros
        self.ke_hist.push(ke_p);
        let h = &self.ke_hist;
        if self.time > 0.3 * self.t_end && h.len() >= 5 {
            let w = &h[h.len() - 5..];
            let (mn, mx) = (w.iter().cloned().fold(f64::MAX, f64::min), w.iter().cloned().fold(f64::MIN, f64::max));
            if mx - mn < 0.004 * self.ke0.max(1e-12) {
                self.t_end = self.time;
            }
        }
        let (e_el, e_ct) = (self.cur_e_el, self.cur_e_ct);
        self.frames.push(Frame {
            time: self.time,
            pos: self.p[..n].iter().map(|q| [q.x as f32, q.y as f32, q.z as f32]).collect(),
            speed: self.p[..n].iter().map(|q| (q.vx * q.vx + q.vy * q.vy + q.vz * q.vz).sqrt() as f32).collect(),
            von_mises: von,
            pressure: press,
            plastic: tens.iter().map(|t| t.1).collect(),
            temp: self.p[..n].iter().map(|q| q.temp as f32).collect(),
            damage: tens.iter().map(|t| t.2).collect(),
            ke_proj: ke_p * self.gfac,
            ke_target: self.kinetic(Some(false)) * self.gfac,
            vx_proj: vx_p / np.max(1.0),
            e_elastic: e_el * self.gfac,
            e_contact: e_ct * self.gfac,
            e_plastic: self.e_pl * self.gfac,
            e_fracture: self.e_fr * self.gfac,
        });
    }

    pub fn outcome(&self) -> Outcome {
        let n = self.n_real;
        let (mut np, mut behind, mut vx, mut tip) = (0.0f64, 0.0f64, 0.0f64, f64::MIN);
        let (mut nt, mut ejected) = (0.0f64, 0.0f64);
        for i in 0..n {
            let q = self.p[i];
            if self.part[i] < self.n_proj {
                np += 1.0;
                vx += q.vx;
                tip = tip.max(q.x);
                if q.x > self.back_x {
                    behind += 1.0;
                }
            } else {
                nt += 1.0;
                if q.x > self.back_x + 2.0 * self.cfg.dx {
                    ejected += 1.0;
                }
            }
        }
        let v_res = vx / np.max(1.0);
        let behind_frac = behind / np.max(1.0);
        let kind = if behind_frac > 0.5 && v_res > 0.0 {
            OutcomeKind::Perforation
        } else if v_res < 0.0 && behind_frac < 0.05 {
            OutcomeKind::Rejected
        } else {
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
