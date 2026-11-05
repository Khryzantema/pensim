use crate::config::Config;
use crate::geometry::Model;
use crate::material::{Material, T_REF};
use rayon::prelude::*;
use std::time::{Duration, Instant};

const TAYLOR_QUINNEY: f64 = 0.9;
/// fração do limite de estabilidade do passo explícito
const DT_SAFETY: f64 = 0.65;

#[derive(Clone, Copy)]
struct Bond {
    j: u32,
    /// 1/comprimento de referência
    ixi: f32,
    /// correção de volume nas bordas do horizonte
    w: f32,
    sp: f32,
    acc: f32,
    /// última força (N, + = tração)
    f: f32,
    alive: bool,
}

#[derive(Clone, Copy, Default)]
struct P {
    x: f64,
    y: f64,
    z: f64,
    vx: f64,
    vy: f64,
    vz: f64,
    temp: f64,
}

#[derive(Clone, Copy, Default)]
struct Out {
    f: [f64; 3],
    d_temp: f64,
    e_el: f64,
    e_ct: f64,
    e_pl: f64,
    e_fr: f64,
    broke: bool,
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
    out: Vec<Out>,
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
    exit_found: bool,
    soft: Vec<f64>,
    contact_on: Vec<bool>,
}

#[derive(Clone, Copy, Default)]
struct GridInfo {
    n: [i64; 3],
}

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
        let model = Model::build(&cfg)?;
        let dx = cfg.dx;
        let vol = dx * dx * dx;
        let horizon = cfg.horizon_m * dx;
        let n = model.pos.len();
        let c_bond: Vec<f64> = model.mats.iter().map(|m| m.micromodulus(horizon)).collect();
        let c_contact = c_bond.iter().cloned().fold(0.0, f64::max);
        let mass: Vec<f64> = model.mats.iter().map(|m| m.rho * vol).collect();
        let speed = cfg.velocity;
        let p: Vec<P> = (0..n)
            .map(|i| {
                let q = model.pos[i];
                let proj = model.part[i] < model.n_proj;
                P {
                    x: q[0], y: q[1], z: q[2], temp: T_REF,
                    vx: if proj { speed * model.dir[0] } else { 0.0 },
                    vy: 0.0,
                    vz: if proj { speed * model.dir[2] } else { 0.0 },
                }
            })
            .collect();
        let mut sim = Sim {
            time: 0.0,
            dt: 0.0,
            step_count: 0,
            frames: vec![],
            out: vec![Out::default(); n],
            p,
            pos0: model.pos,
            part: model.part,
            fixed: model.fixed,
            surface: vec![false; n],
            bonds: vec![vec![]; n],
            initial_bonds: vec![0; n],
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
            exit_found: false,
            soft: vec![1.0; n],
            contact_on: vec![true; n],
            cfg,
        };
        sim.build_bonds(horizon);
        sim.contact_on = vec![true; n]; // contato em todos os nós: filtrar por superfície perde reações durante a penetração
        sim.dt = sim.stable_dt();
        sim.ke0 = sim.kinetic(Some(true));
        Ok(sim)
    }

    pub fn n(&self) -> usize {
        self.p.len()
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
        self.part.clone()
    }
    /// nós de superfície na configuração de referência
    pub fn surface(&self) -> Vec<bool> {
        self.surface.clone()
    }
    pub fn ref_positions(&self) -> Vec<[f32; 3]> {
        self.pos0.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect()
    }

    fn build_grid(&mut self, use_ref: bool, cs_min: f64) {
        let n = self.p.len();
        let get = |i: usize| -> [f64; 3] {
            if use_ref { self.pos0[i] } else { [self.p[i].x, self.p[i].y, self.p[i].z] }
        };
        let (mut mn, mut mx) = ([f64::MAX; 3], [f64::MIN; 3]);
        for i in 0..n {
            let q = get(i);
            if !q.iter().all(|v| v.is_finite()) {
                continue;
            }
            for k in 0..3 {
                mn[k] = mn[k].min(q[k]);
                mx[k] = mx[k].max(q[k]);
            }
        }
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
        for i in 0..n {
            let q = get(i);
            let c = [0, 1, 2].map(|k| (((q[k] - mn[k]) / cs) as i64).clamp(0, nn[k] - 1));
            let id = (c[2] * nn[1] + c[1]) * nn[0] + c[0];
            cell_of[i] = id as u32;
            cell_start[id as usize + 1] += 1;
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
        let bonds: Vec<Vec<Bond>> = (0..self.p.len())
            .into_par_iter()
            .map(|i| {
                let c = cell_coords(self.cell_of[i], &g);
                let mut list = vec![];
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
                                    list.push(Bond { j: j as u32, ixi: (1.0 / xi) as f32, w: w as f32, sp: 0.0, acc: 0.0, f: 0.0, alive: true });
                                }
                            }
                        }
                    }
                }
                list
            })
            .collect();
        self.initial_bonds = bonds.iter().map(|b| b.len() as u32).collect();
        self.bonds = bonds;
        // nós de superfície: menos ligações que o máximo da peça
        let nparts = self.mats.len();
        let mut maxb = vec![0u32; nparts];
        for i in 0..self.p.len() {
            let p = self.part[i] as usize;
            maxb[p] = maxb[p].max(self.initial_bonds[i]);
        }
        for i in 0..self.p.len() {
            self.surface[i] = (self.initial_bonds[i] as f64) < 0.97 * maxb[self.part[i] as usize] as f64;
        }
    }

    fn stable_dt(&self) -> f64 {
        let v = self.vol;
        let mut dt = f64::MAX;
        for i in 0..self.p.len() {
            let b = self.part[i] as usize;
            let c = self.c_bond[b];
            let mut s: f64 = self.bonds[i].iter().map(|bd| c * bd.w as f64 * v * bd.ixi as f64).sum();
            s += 12.0 * self.c_contact * v / (0.8 * self.cfg.dx);
            dt = dt.min(DT_SAFETY * (2.0 * self.mats[b].rho / s).sqrt());
        }
        dt
    }

    fn kinetic(&self, proj: Option<bool>) -> f64 {
        self.p
            .iter()
            .enumerate()
            .filter(|(i, _)| proj.map_or(true, |pr| (self.part[*i] < self.n_proj) == pr))
            .map(|(i, q)| 0.5 * self.mass[self.part[i] as usize] * (q.vx * q.vx + q.vy * q.vy + q.vz * q.vz))
            .sum()
    }

    /// Um passo explícito, paralelo sobre os nós.
    pub fn step(&mut self) {
        let dt = self.dt;
        let vol = self.vol;
        let dx = self.cfg.dx;
        self.build_grid(false, dx);
        let g = self.grid;
        {
            let p = &self.p;
            let part = &self.part;
            let fixed = &self.fixed;
            let soft = &self.soft;
            let contact_on = &self.contact_on;
            let mats = &self.mats;
            let c_bond = &self.c_bond;
            let c_ct = self.c_contact;
            let cell_start = &self.cell_start;
            let cell_items = &self.cell_items;
            let cell_of = &self.cell_of;
            self.bonds
                .par_iter_mut()
                .zip(self.out.par_iter_mut())
                .enumerate()
                .with_min_len(32)
                .for_each(|(i, (bonds, out))| {
                    if fixed[i] {
                        *out = Out::default();
                        return;
                    }
                    let me = p[i];
                    let b = part[i] as usize;
                    let mat = &mats[b];
                    let c = c_bond[b];
                    let mut o = Out::default();
                    let v2 = vol * vol;
                    let fail = mat.fail_stretch;
                    let sy0 = mat.yield_strength / mat.young;
                    let hard = mat.hardening / mat.young;
                    let cvv = c * v2;
                    let soft_i = soft[i];
                    let (mut fx, mut fy, mut fz) = (0.0f64, 0.0f64, 0.0f64);
                    for bd in bonds.iter_mut() {
                        if !bd.alive {
                            continue;
                        }
                        let q = &p[bd.j as usize];
                        let (rx, ry, rz) = (q.x - me.x, q.y - me.y, q.z - me.z);
                        let r = (rx * rx + ry * ry + rz * rz).sqrt().max(1e-12);
                        let ixi = bd.ixi as f64;
                        let s = r * ixi - 1.0;
                        let sp = bd.sp as f64;
                        let w = bd.w as f64;
                        if s > fail {
                            bd.alive = false;
                            let s_el = s - sp;
                            o.e_fr += 0.25 * cvv * s_el * s_el * w / ixi;
                            bd.f = 0.0;
                            o.broke = true;
                            continue;
                        }
                        let soft_b = 0.5 * (soft_i + soft[bd.j as usize]);
                        let sy_t = (sy0 + hard * bd.acc as f64) * soft_b;
                        let mut s_el = s - sp;
                        if s_el > sy_t || s_el < -sy_t * mat.comp_factor {
                            let lim = if s_el > 0.0 { sy_t } else { -sy_t * mat.comp_factor };
                            let dsp = s_el - lim;
                            bd.sp = (sp + dsp) as f32;
                            bd.acc += dsp.abs() as f32;
                            s_el = lim;
                            let de = cvv * lim.abs() * dsp.abs() * w / ixi;
                            o.e_pl += 0.5 * de;
                            o.d_temp += TAYLOR_QUINNEY * 0.5 * de / (mat.rho * mat.cp * vol);
                        }
                        let f = cvv * s_el * w;
                        bd.f = f as f32;
                        let k = f / r;
                        fx += k * rx;
                        fy += k * ry;
                        fz += k * rz;
                        o.e_el += 0.25 * cvv * s_el * s_el * w / ixi;
                    }
                    o.f = [fx, fy, fz];
                    // contato por penalidade: só nós de superfície ou com ligações rompidas
                    if contact_on[i] {
                        let c0 = cell_coords(cell_of[i], &g);
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
                                        if j == i {
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
                                        let f = c_ct * s * v2;
                                        o.f[0] += f * rx / r;
                                        o.f[1] += f * ry / r;
                                        o.f[2] += f * rz / r;
                                        o.e_ct += 0.25 * c_ct * s * s * rc * v2;
                                    }
                                }
                            }
                        }
                    }
                    *out = o;
                });
        }
        {
            let part = &self.part;
            let fixed = &self.fixed;
            let mass = &self.mass;
            let mats = &self.mats;
            self.p
                .par_iter_mut()
                .zip(self.out.par_iter())
                .zip(self.soft.par_iter_mut())
                .zip(self.contact_on.par_iter_mut())
                .enumerate()
                .with_min_len(256)
                .for_each(|(i, (((q, o), soft), con))| {
                    q.temp += o.d_temp;
                    let m = &mats[part[i] as usize];
                    let theta = ((q.temp - T_REF) / (m.t_melt - T_REF)).clamp(0.0, 1.0);
                    *soft = (1.0 - theta).max(0.03);
                    if o.broke {
                        *con = true;
                    }
                    if fixed[i] {
                        q.vx = 0.0;
                        q.vy = 0.0;
                        q.vz = 0.0;
                        return;
                    }
                    let mm = mass[part[i] as usize];
                    q.vx += o.f[0] / mm * dt;
                    q.vy += o.f[1] / mm * dt;
                    q.vz += o.f[2] / mm * dt;
                    q.x += q.vx * dt;
                    q.y += q.vy * dt;
                    q.z += q.vz * dt;
                });
        }
        let (dpl, dfr) = self.out.iter().fold((0.0, 0.0), |a, o| (a.0 + o.e_pl, a.1 + o.e_fr));
        self.e_pl += dpl;
        self.e_fr += dfr;
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
        for (i, q) in self.p.iter().enumerate() {
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
        let n = self.p.len();
        let vol = self.vol;
        let tens: Vec<([f32; 6], f32, f32)> = (0..n)
            .into_par_iter()
            .map(|i| {
                let me = self.p[i];
                let mut s = [0.0f64; 6]; // xx yy zz xy yz zx
                let (mut acc, mut alive) = (0.0f64, 0u32);
                for bd in &self.bonds[i] {
                    acc += bd.acc as f64;
                    if !bd.alive {
                        continue;
                    }
                    alive += 1;
                    let q = self.p[bd.j as usize];
                    let (rx, ry, rz) = (q.x - me.x, q.y - me.y, q.z - me.z);
                    let r = (rx * rx + ry * ry + rz * rz).sqrt().max(1e-12);
                    let k = 0.5 * bd.f as f64 / r / vol;
                    s[0] += k * rx * rx;
                    s[1] += k * ry * ry;
                    s[2] += k * rz * rz;
                    s[3] += k * rx * ry;
                    s[4] += k * ry * rz;
                    s[5] += k * rz * rx;
                }
                let ib = self.initial_bonds[i].max(1) as f64;
                (
                    [s[0] as f32, s[1] as f32, s[2] as f32, s[3] as f32, s[4] as f32, s[5] as f32],
                    (acc / ib) as f32,
                    1.0 - alive as f32 / ib as f32,
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
        let e_el: f64 = self.out.iter().map(|o| o.e_el).sum();
        let e_ct: f64 = self.out.iter().map(|o| o.e_ct).sum();
        self.frames.push(Frame {
            time: self.time,
            pos: self.p.iter().map(|q| [q.x as f32, q.y as f32, q.z as f32]).collect(),
            speed: self.p.iter().map(|q| (q.vx * q.vx + q.vy * q.vy + q.vz * q.vz).sqrt() as f32).collect(),
            von_mises: von,
            pressure: press,
            plastic: tens.iter().map(|t| t.1).collect(),
            temp: self.p.iter().map(|q| q.temp as f32).collect(),
            damage: tens.iter().map(|t| t.2).collect(),
            ke_proj: ke_p,
            ke_target: self.kinetic(Some(false)),
            vx_proj: vx_p / np.max(1.0),
            e_elastic: e_el,
            e_contact: e_ct,
            e_plastic: self.e_pl,
            e_fracture: self.e_fr,
        });
    }

    pub fn outcome(&self) -> Outcome {
        let n = self.p.len();
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
        let e_el: f64 = self.out.iter().map(|o| o.e_el).sum();
        let e_ct: f64 = self.out.iter().map(|o| o.e_ct).sum();
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
