//! Núcleo novo: elementos finitos explícitos Lagrangianos (projeto em `docs/NOVO-NUCLEO.md`).
//!
//! Fase 1: malha de hexaedros, elemento de 1 ponto com hourglass por rigidez
//! (Flanagan & Belytschko 1981), taxa de Jaumann, Johnson-Cook com retorno radial, EOS
//! Mie-Grüneisen, viscosidade artificial volumétrica, diferenças centrais, parede rígida e
//! condições de simetria. Ainda sem contato entre corpos e sem erosão.
//!
//! Estado em `f64` nesta fase (correção primeiro); a troca para `f32` é da fase 6.
pub mod build;
pub mod contact;
pub mod run;
pub mod material;
pub mod mesh;

use material::FemMat;
use mesh::{HexMesh, NODE_SIGNS};
use rayon::prelude::*;
use std::sync::atomic::{AtomicUsize, Ordering};

const NONE: u32 = u32::MAX;
/// Erosão de segurança: elemento cujo passo estável cai abaixo desta fração do inicial
const EROSION_DT_FRAC: f64 = 0.1;

/// Coeficientes de viscosidade artificial (padrão dos códigos explícitos): q = ρ l (Q1 l ε̇v² − Q2 c ε̇v)
const Q1: f64 = 1.5;
const Q2: f64 = 0.06;
/// Rigidez do hourglass (fração da rigidez do elemento)
const HG_KAPPA: f64 = 0.1;
fn hg_kappa() -> f64 {
    static K: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *K.get_or_init(|| std::env::var("PENSIM_HG").ok().and_then(|v| v.parse().ok()).unwrap_or(HG_KAPPA))
}
/// Fração do limite de estabilidade
const DT_SCALE: f64 = 0.9;

/// Velocidade prescrita numa componente de um conjunto de nós: v(t) = amp · rampa(t).
#[derive(Clone, Debug)]
pub struct Prescribed {
    pub nodes: Vec<u32>,
    pub axis: usize,
    pub amp: f64,
    /// tempo de subida (rampa seno² de 0 a amp) [s]
    pub rise: f64,
}

impl Prescribed {
    pub fn value(&self, t: f64) -> f64 {
        if t >= self.rise {
            self.amp
        } else {
            let s = (0.5 * std::f64::consts::PI * t / self.rise).sin();
            self.amp * s * s
        }
    }
}

/// Parede rígida plana, sem atrito: o corpo não atravessa `x[axis] = pos` vindo do lado `side`.
#[derive(Clone, Copy, Debug)]
pub struct Wall {
    pub axis: usize,
    pub pos: f64,
    /// +1: corpo do lado positivo (não pode ir abaixo de pos); −1: do lado negativo
    pub side: f64,
}

pub struct FemModel {
    pub mesh: HexMesh,
    /// material por peça (`mesh.part` indexa aqui)
    pub mats: Vec<FemMat>,
    /// velocidade inicial por nó
    pub v0: Vec<[f64; 3]>,
    pub prescribed: Vec<Prescribed>,
    pub walls: Vec<Wall>,
    /// coeficiente de atrito de Coulomb no contato
    pub friction: f64,
    /// liga contato e erosão (desligado nos testes de verificação da fase 1)
    pub contact: bool,
}

impl FemModel {
    pub fn new(mesh: HexMesh, mats: Vec<FemMat>, v0: Vec<[f64; 3]>) -> Self {
        FemModel { mesh, mats, v0, prescribed: vec![], walls: vec![], friction: 0.0, contact: false }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Elem {
    /// tensão desviadora (xx, yy, zz, xy, yz, zx)
    s: [f64; 6],
    /// pressão (EOS, compressão > 0)
    p: f64,
    /// viscosidade artificial
    q: f64,
    v0: f64,
    vol: f64,
    mass: f64,
    /// energia interna [J]
    e_int: f64,
    eps_p: f64,
    eps_rate: f64,
    temp: f64,
    /// dano de Johnson-Cook acumulado (≥ 1 apaga o elemento)
    dmg: f64,
    alive: bool,
    /// passo estável inicial do elemento (critério de erosão por distorção)
    dt0: f64,
    /// forças generalizadas de hourglass (4 modos × 3)
    hg: [[f64; 3]; 4],
    e_hg: f64,
    /// comprimento característico atual
    lc: f64,
    /// velocidade do som atual
    c: f64,
}

/// Energias globais [J].
#[derive(Clone, Copy, Debug, Default)]
pub struct Energy {
    pub kinetic: f64,
    pub internal: f64,
    pub hourglass: f64,
    /// trabalho feito pelas condições de contorno sobre o corpo (paredes, velocidades prescritas)
    pub external: f64,
    /// energia guardada/dissipada no contato (penalidade + atrito)
    pub contact: f64,
    pub plastic: f64,
}

impl Energy {
    /// Erro de balanço: (cinética + interna + hourglass − externa) − total inicial.
    pub fn balance(&self, e0: f64) -> f64 {
        self.kinetic + self.internal + self.hourglass + self.contact - self.external - e0
    }
}

pub struct Fem {
    pub mesh: HexMesh,
    pub mats: Vec<FemMat>,
    pub x: Vec<[f64; 3]>,
    /// velocidade no meio passo (n − ½ antes do passo, n + ½ depois)
    pub v: Vec<[f64; 3]>,
    mass: Vec<f64>,
    f: Vec<[f64; 3]>,
    elem: Vec<Elem>,
    /// forças de cada elemento nos seus 8 nós (escritas em paralelo, juntadas por nó)
    ef: Vec<[[f64; 3]; 8]>,
    /// nó → lista de (elemento·8 + nó local), formato CSR
    adj_ptr: Vec<u32>,
    adj: Vec<u32>,
    prescribed: Vec<Prescribed>,
    walls: Vec<Wall>,
    pub time: f64,
    pub dt: f64,
    pub steps: u64,
    pub energy: Energy,
    pub e0: f64,
    /// vizinho do elemento por face (NONE na borda)
    face_nb: Vec<[u32; 6]>,
    pub alive: Vec<bool>,
    surf: Vec<contact::SurfFace>,
    cnodes: Vec<u32>,
    surf_dirty: bool,
    fc: Vec<[f64; 3]>,
    /// penetração inicial guardada por nó (NaN = sem contato no passo anterior)
    c_offset: Vec<f64>,
    friction: f64,
    use_contact: bool,
    eroded: AtomicUsize,
    pub n_eroded: usize,
    /// elementos erodidos por motivo: [dano, frágil, distorção, volume]
    erode_why: [AtomicUsize; 4],
}

impl Fem {
    pub fn new(model: FemModel) -> Fem {
        let FemModel { mesh, mats, v0, prescribed, walls, friction, contact } = model;
        let n = mesh.n_nodes();
        let ne = mesh.n_elems();
        assert_eq!(v0.len(), n);
        let mut mass = vec![0.0; n];
        let mut elem = vec![Elem::default(); ne];
        for (e, h) in mesh.hex.iter().enumerate() {
            let m = &mats[mesh.part[e] as usize];
            let vol = mesh::hex_volume(&mesh.corners(h));
            assert!(vol > 0.0, "elemento {e} com volume {vol}");
            let em = m.rho * vol;
            for &k in h {
                mass[k as usize] += em / 8.0;
            }
            elem[e] = Elem { v0: vol, vol, mass: em, temp: m.t_room, alive: true, ..Default::default() };
        }
        // adjacência nó → (elemento, nó local)
        let mut cnt = vec![0u32; n + 1];
        for h in &mesh.hex {
            for &k in h {
                cnt[k as usize + 1] += 1;
            }
        }
        for i in 0..n {
            cnt[i + 1] += cnt[i];
        }
        let mut fill = cnt.clone();
        let mut adj = vec![0u32; cnt[n] as usize];
        for (e, h) in mesh.hex.iter().enumerate() {
            for (ln, &k) in h.iter().enumerate() {
                adj[fill[k as usize] as usize] = (e * 8 + ln) as u32;
                fill[k as usize] += 1;
            }
        }
        // vizinhos por face
        let mut face_nb = vec![[NONE; 6]; ne];
        {
            let mut seen: std::collections::HashMap<[u32; 4], (u32, u8)> = std::collections::HashMap::with_capacity(3 * ne);
            for (e, h) in mesh.hex.iter().enumerate() {
                for (f, fl) in mesh::FACES.iter().enumerate() {
                    let mut k = fl.map(|l| h[l]);
                    k.sort_unstable();
                    if let Some((e2, f2)) = seen.remove(&k) {
                        face_nb[e][f] = e2;
                        face_nb[e2 as usize][f2 as usize] = e as u32;
                    } else {
                        seen.insert(k, (e as u32, f as u8));
                    }
                }
            }
        }
        let x = mesh.x.clone();
        let mut s = Fem {
            mesh,
            mats,
            x,
            v: v0,
            mass,
            f: vec![[0.0; 3]; n],
            elem,
            ef: vec![[[0.0; 3]; 8]; ne],
            adj_ptr: cnt,
            adj,
            prescribed,
            walls,
            time: 0.0,
            dt: 0.0,
            steps: 0,
            energy: Energy::default(),
            e0: 0.0,
            face_nb,
            alive: vec![true; ne],
            surf: vec![],
            cnodes: vec![],
            surf_dirty: true,
            fc: vec![[0.0; 3]; n],
            c_offset: vec![f64::NAN; n],
            friction,
            use_contact: contact,
            eroded: AtomicUsize::new(0),
            n_eroded: 0,
            erode_why: Default::default(),
        };
        s.apply_bc_velocity(0.0);
        // forças e passo iniciais (tensão zero; só fixa lc, c e dt)
        s.element_pass(0.0, true);
        for e in &mut s.elem {
            e.dt0 = e.lc / e.c.max(1e-9);
        }
        s.assemble();
        if s.use_contact {
            s.contact_pass();
        }
        s.dt = s.stable_dt();
        s.energy.kinetic = s.kinetic();
        s.e0 = s.energy.kinetic;
        s
    }

    pub fn n_nodes(&self) -> usize {
        self.x.len()
    }
    pub fn n_elems(&self) -> usize {
        self.elem.len()
    }
    pub fn node_mass(&self, i: usize) -> f64 {
        self.mass[i]
    }

    fn kinetic(&self) -> f64 {
        self.v.par_iter().zip(self.mass.par_iter()).map(|(v, m)| 0.5 * m * (v[0] * v[0] + v[1] * v[1] + v[2] * v[2])).sum()
    }

    fn stable_dt(&self) -> f64 {
        // NaN propaga (f64::min o esconderia)
        let m = self.elem.par_iter().filter(|e| e.alive).map(|e| e.lc / e.c.max(1e-9)).reduce(|| f64::INFINITY, |a, b| if a.is_nan() || b.is_nan() { f64::NAN } else { a.min(b) });
        DT_SCALE * m
    }

    /// Velocidades fixas (simetria, prescritas).
    fn apply_bc_velocity(&mut self, t: f64) {
        let sym = &self.mesh.sym;
        self.v.par_iter_mut().zip(sym.par_iter()).for_each(|(v, &b)| {
            for a in 0..3 {
                if b & (1 << a) != 0 {
                    v[a] = 0.0;
                }
            }
        });
        for p in &self.prescribed {
            let val = p.value(t);
            for &k in &p.nodes {
                self.v[k as usize][p.axis] = val;
            }
        }
    }

    /// Um passo de tempo (diferenças centrais).
    pub fn step(&mut self) {
        let dt = self.dt;
        let t_half = self.time + 0.5 * dt;
        let t_new = self.time + dt;
        // energia cinética no instante n (média dos meio-passos)
        let v_old = self.v.clone();
        // 1. velocidades n+½ (forças de n)
        {
            let (f, m) = (&self.f, &self.mass);
            self.v.par_iter_mut().enumerate().for_each(|(i, v)| {
                let inv = 1.0 / m[i];
                for a in 0..3 {
                    v[a] += f[i][a] * inv * dt;
                }
            });
        }
        // trabalho externo das condições de velocidade: o que a restrição tirou/pôs de quantidade
        // de movimento, vezes a velocidade média (impulso · v)
        let v_free = self.v.clone();
        self.apply_bc_velocity(t_half);
        let mut w_ext = 0.0;
        // paredes rígidas: o nó não atravessa no fim do passo
        for w in &self.walls {
            for i in 0..self.x.len() {
                let xn = self.x[i][w.axis] + self.v[i][w.axis] * dt;
                if (xn - w.pos) * w.side < 0.0 {
                    self.v[i][w.axis] = (w.pos - self.x[i][w.axis]) / dt;
                }
            }
        }
        for i in 0..self.x.len() {
            for a in 0..3 {
                let dv = self.v[i][a] - v_free[i][a];
                if dv != 0.0 {
                    // impulso m·dv aplicado com velocidade média (v_old + v_new)/2
                    w_ext += self.mass[i] * dv * 0.5 * (v_old[i][a] + self.v[i][a]);
                }
            }
        }
        // 2. posições n+1
        {
            let v = &self.v;
            self.x.par_iter_mut().zip(v.par_iter()).for_each(|(x, v)| {
                for a in 0..3 {
                    x[a] += v[a] * dt;
                }
            });
        }
        // 3. elementos: taxa de deformação com v n+½ e geometria n+1 → tensão n+1 → forças n+1
        self.element_pass(dt, false);
        let er = self.eroded.swap(0, Ordering::Relaxed);
        if er > 0 {
            self.n_eroded += er;
            self.surf_dirty = true;
            for (a, e) in self.alive.iter_mut().zip(&self.elem) {
                *a = e.alive;
            }
        }
        self.assemble();
        if self.use_contact {
            // trabalho do contato no passo com a força média (n e n+1), como nos elementos
            let fc_old = std::mem::take(&mut self.fc);
            self.fc = vec![[0.0; 3]; fc_old.len()];
            self.contact_pass();
            let w: f64 = fc_old
                .par_iter()
                .zip(self.fc.par_iter())
                .zip(self.v.par_iter())
                .map(|((a, b), v)| 0.5 * ((a[0] + b[0]) * v[0] + (a[1] + b[1]) * v[1] + (a[2] + b[2]) * v[2]) * dt)
                .sum();
            self.energy.contact -= w;
        }
        // energia
        let ke = self.kinetic_mid(&v_old);
        self.energy.kinetic = ke;
        self.energy.external += w_ext;
        let (ie, hg, pl) = self.elem.par_iter().map(|e| (e.e_int, e.e_hg, 0.0)).reduce(|| (0.0, 0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1, a.2 + b.2));
        self.energy.internal = ie;
        self.energy.hourglass = hg;
        let _ = pl;
        self.time = t_new;
        self.steps += 1;
        self.dt = self.stable_dt();
    }

    fn kinetic_mid(&self, v_old: &[[f64; 3]]) -> f64 {
        // ½ m v(n−½)·v(n+½): energia cinética centrada em n (consistente com diferenças centrais)
        self.v
            .par_iter()
            .zip(v_old.par_iter())
            .zip(self.mass.par_iter())
            .map(|((a, b), m)| 0.5 * m * (a[0] * b[0] + a[1] * b[1] + a[2] * b[2]))
            .sum()
    }

    /// Energia cinética no fim do passo (½ m v²), para relatórios.
    pub fn kinetic_now(&self) -> f64 {
        self.kinetic()
    }

    pub fn run_until(&mut self, t_end: f64) {
        while self.time < t_end {
            self.step();
            assert!(self.dt.is_finite() && self.dt > 0.0 && self.energy.kinetic.is_finite(), "simulação instável no passo {} (t = {:.3e} s)", self.steps, self.time);
        }
    }

    /// Junta as forças dos elementos em cada nó (sem condição de corrida: cada nó soma as suas).
    fn assemble(&mut self) {
        let (ptr, adj, ef) = (&self.adj_ptr, &self.adj, &self.ef);
        self.f.par_iter_mut().enumerate().for_each(|(i, f)| {
            let mut acc = [0.0; 3];
            for &k in &adj[ptr[i] as usize..ptr[i + 1] as usize] {
                let v = ef[(k / 8) as usize][(k % 8) as usize];
                acc[0] += v[0];
                acc[1] += v[1];
                acc[2] += v[2];
            }
            *f = acc;
        });
    }

    /// Atualiza tensão e calcula as forças nodais de cada elemento.
    fn element_pass(&mut self, dt: f64, init: bool) {
        let (x, v, hexes, parts, mats) = (&self.x, &self.v, &self.mesh.hex, &self.mesh.part, &self.mats);
        let eroded = &self.eroded;
        let why = &self.erode_why;
        let erosion = self.use_contact;
        self.elem.par_iter_mut().zip(self.ef.par_iter_mut()).enumerate().for_each(|(e, (st, fe))| {
            if !st.alive {
                return;
            }
            let h = &hexes[e];
            let m = &mats[parts[e] as usize];
            let xe: [[f64; 3]; 8] = std::array::from_fn(|k| x[h[k] as usize]);
            let ve: [[f64; 3]; 8] = std::array::from_fn(|k| v[h[k] as usize]);
            let (b, vol) = b_matrix(&xe);
            let vol = vol.max(1e-30);
            // gradiente de velocidade L_ab = Σ v_ia B_ib
            let mut l = [[0.0; 3]; 3];
            for k in 0..8 {
                for a in 0..3 {
                    for c in 0..3 {
                        l[a][c] += ve[k][a] * b[k][c];
                    }
                }
            }
            let d = [l[0][0], l[1][1], l[2][2], 0.5 * (l[0][1] + l[1][0]), 0.5 * (l[1][2] + l[2][1]), 0.5 * (l[2][0] + l[0][2])];
            let w = [0.5 * (l[0][1] - l[1][0]), 0.5 * (l[1][2] - l[2][1]), 0.5 * (l[2][0] - l[0][2])]; // w_xy, w_yz, w_zx
            let lc = char_length(&xe, vol);
            let mu = st.v0 / vol - 1.0;
            let rho = st.mass / vol;
            if !init {
                let dv = d[0] + d[1] + d[2];
                let s_old = st.s;
                let pq_old = st.p + st.q;
                // --- desviadora: rotação de Jaumann + incremento elástico
                let mut s = jaumann(st.s, w, dt);
                let tr3 = dv / 3.0;
                let g2 = 2.0 * m.g * dt;
                s[0] += g2 * (d[0] - tr3);
                s[1] += g2 * (d[1] - tr3);
                s[2] += g2 * (d[2] - tr3);
                s[3] += g2 * d[3];
                s[4] += g2 * d[4];
                s[5] += g2 * d[5];
                // --- retorno radial (Johnson-Cook)
                let j2 = 0.5 * (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]) + s[3] * s[3] + s[4] * s[4] + s[5] * s[5];
                let seq = (3.0 * j2).sqrt();
                let mut dep = 0.0;
                let sy0 = m.yield_stress(st.eps_p, st.eps_rate, st.temp);
                if seq > sy0 {
                    // uma iteração de Newton com a inclinação do encruamento
                    let hs = m.hardening_slope(st.eps_p);
                    dep = (seq - sy0) / (3.0 * m.g + hs);
                    let sy1 = m.yield_stress(st.eps_p + dep, st.eps_rate, st.temp);
                    let r = (sy1 / seq).min(1.0);
                    for c in &mut s {
                        *c *= r;
                    }
                }
                st.s = s;
                st.eps_p += dep;
                st.eps_rate = if dt > 0.0 { dep / dt } else { 0.0 };
                // --- pressão (EOS) com a energia interna por volume inicial
                st.p = m.pressure(mu, st.e_int / st.v0);
                // --- viscosidade artificial (só em compressão)
                st.c = sound_speed(m, mu, rho);
                st.q = if dv < 0.0 { rho * lc * (Q1 * lc * dv * dv - Q2 * st.c * dv) } else { 0.0 };
                // --- energia interna: ∫ σ:D dV dt com σ médio (σ = s − (p+q) I)
                let s_mid: [f64; 6] = std::array::from_fn(|k| 0.5 * (s_old[k] + st.s[k]));
                let pq_mid = 0.5 * (pq_old + st.p + st.q);
                let sd = s_mid[0] * d[0] + s_mid[1] * d[1] + s_mid[2] * d[2] + 2.0 * (s_mid[3] * d[3] + s_mid[4] * d[4] + s_mid[5] * d[5]);
                let de = (sd - pq_mid * dv) * vol * dt;
                st.e_int += de;
                // calor do trabalho plástico
                if dep > 0.0 {
                    let wp = sy0.max(0.0) * dep; // por volume
                    st.temp += m.taylor_quinney * wp / (rho * m.cp);
                }
                // pressão consistente com a energia nova
                st.p = m.pressure(mu, st.e_int / st.v0);
                // --- dano de Johnson-Cook: Δεp / εf(triaxialidade, taxa, temperatura)
                if dep > 0.0 {
                    let j2n = 0.5 * (st.s[0] * st.s[0] + st.s[1] * st.s[1] + st.s[2] * st.s[2]) + st.s[3] * st.s[3] + st.s[4] * st.s[4] + st.s[5] * st.s[5];
                    let seq_n = (3.0 * j2n).sqrt().max(1.0);
                    let triax = (-st.p / seq_n).clamp(-5.0, 5.0);
                    if let Some(ef) = m.failure_strain(triax, st.eps_rate, st.temp) {
                        st.dmg += dep / ef;
                    }
                }
                // --- erosão: dano, frágil, distorção extrema, volume invertido
                if erosion {
                    let brittle = m.brittle_stress.is_some_and(|sb| max_principal(&st.s, st.p) > sb);
                    let dt_e = lc / st.c.max(1e-9);
                    let reason = if st.dmg >= 1.0 {
                        Some(0)
                    } else if brittle {
                        Some(1)
                    } else if dt_e < EROSION_DT_FRAC * st.dt0 {
                        Some(2)
                    } else if vol <= 0.0 {
                        Some(3)
                    } else {
                        None
                    };
                    if let Some(r) = reason {
                        why[r].fetch_add(1, Ordering::Relaxed);
                        st.alive = false;
                        *fe = [[0.0; 3]; 8];
                        eroded.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                }
            } else {
                st.c = sound_speed(m, mu, rho);
            }
            st.vol = vol;
            st.lc = lc;
            // --- forças internas: f_i = −V σ·B_i
            let ptot = st.p + st.q;
            let sig = [st.s[0] - ptot, st.s[1] - ptot, st.s[2] - ptot, st.s[3], st.s[4], st.s[5]];
            for k in 0..8 {
                let bk = b[k];
                fe[k] = [
                    -vol * (sig[0] * bk[0] + sig[3] * bk[1] + sig[5] * bk[2]),
                    -vol * (sig[3] * bk[0] + sig[1] * bk[1] + sig[4] * bk[2]),
                    -vol * (sig[5] * bk[0] + sig[4] * bk[1] + sig[2] * bk[2]),
                ];
            }
            // --- hourglass por rigidez (Flanagan-Belytschko)
            let gam = gamma(&xe, &b);
            let bb: f64 = b.iter().map(|r| r[0] * r[0] + r[1] * r[1] + r[2] * r[2]).sum();
            let k_hg = hg_kappa() * (m.bulk_tangent(mu.max(0.0)) + 4.0 / 3.0 * m.g) * bb * vol / 8.0;
            if !init {
                for al in 0..4 {
                    let mut qa = [0.0; 3];
                    for k in 0..8 {
                        for a in 0..3 {
                            qa[a] += gam[al][k] * ve[k][a];
                        }
                    }
                    let old = st.hg[al];
                    for a in 0..3 {
                        st.hg[al][a] += k_hg * qa[a] * dt;
                    }
                    // energia de hourglass: força média · taxa do modo
                    for a in 0..3 {
                        st.e_hg += 0.5 * (old[a] + st.hg[al][a]) * qa[a] * dt;
                    }
                }
            }
            for al in 0..4 {
                for k in 0..8 {
                    for a in 0..3 {
                        fe[k][a] -= st.hg[al][a] * gam[al][k];
                    }
                }
            }
        });
    }

    /// Refaz a lista de faces externas e de nós de contato (depois de erosão).
    fn rebuild_surface(&mut self) {
        let (alive, nb, sym, hex) = (&self.alive, &self.face_nb, &self.mesh.sym, &self.mesh.hex);
        self.surf = (0..self.elem.len())
            .into_par_iter()
            .filter(|&e| alive[e])
            .flat_map_iter(|e| {
                (0..6u8).filter_map(move |f| {
                    let o = nb[e][f as usize];
                    if o != NONE && alive[o as usize] {
                        return None;
                    }
                    // face num plano de simetria não é superfície de contato
                    let common = mesh::FACES[f as usize].iter().fold(0xFFu8, |a, &l| a & sym[hex[e][l] as usize]);
                    if common & 6 != 0 {
                        return None;
                    }
                    Some(contact::SurfFace { elem: e as u32, face: f })
                })
            })
            .collect();
        let n = self.x.len();
        let mut flag = vec![false; n];
        for sf in &self.surf {
            for &l in &mesh::FACES[sf.face as usize] {
                flag[self.mesh.hex[sf.elem as usize][l] as usize] = true;
            }
        }
        // partículas: nós sem nenhum elemento vivo
        for i in 0..n {
            if !flag[i] {
                let live = self.adj[self.adj_ptr[i] as usize..self.adj_ptr[i + 1] as usize].iter().any(|&a| self.alive[(a / 8) as usize]);
                if !live {
                    flag[i] = true;
                }
            }
        }
        self.cnodes = (0..n as u32).filter(|&i| flag[i as usize]).collect();
        self.surf_dirty = false;
    }

    /// Forças de contato do instante atual, somadas em `f` e guardadas em `fc`.
    fn contact_pass(&mut self) {
        if self.surf_dirty {
            self.rebuild_surface();
        }
        let bulk: Vec<f64> = (0..self.elem.len()).map(|e| self.mats[self.mesh.part[e] as usize].bulk()).collect();
        let vol: Vec<f64> = self.elem.iter().map(|e| e.vol).collect();
        let lc: Vec<f64> = self.elem.iter().map(|e| e.lc).collect();
        let hits = contact::forces(&contact::ContactInput {
            x: &self.x,
            v: &self.v,
            mass: &self.mass,
            hex: &self.mesh.hex,
            alive: &self.alive,
            bulk: &bulk,
            vol: &vol,
            lc: &lc,
            surf: &self.surf,
            nodes: &self.cnodes,
            adj_ptr: &self.adj_ptr,
            adj: &self.adj,
            friction: self.friction,
            dt: self.dt.max(1e-15),
            prev_offset: &self.c_offset,
        });
        self.c_offset.par_iter_mut().for_each(|o| *o = f64::NAN);
        for h in &hits {
            self.c_offset[h.node as usize] = h.offset;
        }
        self.fc.par_iter_mut().for_each(|f| *f = [0.0; 3]);
        for h in &hits {
            for a in 0..3 {
                self.fc[h.node as usize][a] += h.f[a];
                for k in 0..4 {
                    self.fc[h.fnodes[k] as usize][a] -= h.w[k] * h.f[a];
                }
            }
        }
        let fc = &self.fc;
        self.f.par_iter_mut().zip(fc.par_iter()).for_each(|(f, c)| {
            f[0] += c[0];
            f[1] += c[1];
            f[2] += c[2];
        });
    }

    // ----------------------------------------------------------------- consultas

    /// Campos por nó (média dos elementos vivos em volta): von Mises, pressão, def. plástica,
    /// temperatura, dano. Nó sem elemento vivo (partícula) fica com dano 1.
    pub fn node_fields(&self) -> [Vec<f32>; 5] {
        let (ptr, adj, el) = (&self.adj_ptr, &self.adj, &self.elem);
        let r: Vec<[f32; 5]> = (0..self.x.len())
            .into_par_iter()
            .map(|i| {
                let (mut acc, mut n) = ([0.0f64; 5], 0.0);
                let mut t_any = 0.0f64;
                for &a in &adj[ptr[i] as usize..ptr[i + 1] as usize] {
                    let st = &el[(a / 8) as usize];
                    t_any = t_any.max(st.temp);
                    if !st.alive {
                        continue;
                    }
                    let s = &st.s;
                    let vm = (1.5 * (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]) + 3.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5])).sqrt();
                    acc[0] += vm;
                    acc[1] += st.p;
                    acc[2] += st.eps_p;
                    acc[3] += st.temp;
                    acc[4] += st.dmg.min(1.0);
                    n += 1.0;
                }
                if n == 0.0 {
                    [0.0, 0.0, 0.0, t_any as f32, 1.0]
                } else {
                    acc.map(|v| (v / n) as f32)
                }
            })
            .collect();
        let mut out: [Vec<f32>; 5] = Default::default();
        for (k, o) in out.iter_mut().enumerate() {
            *o = r.iter().map(|v| v[k]).collect();
        }
        out
    }

    /// Elementos vivos como bits (bit e de `out[e / 8]`).
    pub fn alive_bits(&self) -> Vec<u8> {
        let mut b = vec![0u8; self.elem.len().div_ceil(8)];
        for (e, st) in self.elem.iter().enumerate() {
            if st.alive {
                b[e / 8] |= 1 << (e % 8);
            }
        }
        b
    }

    /// Massa dos nós (por nó).
    pub fn masses(&self) -> &[f64] {
        &self.mass
    }

    /// Elementos erodidos por motivo: [dano, frágil, distorção, volume].
    pub fn erosion_reasons(&self) -> [usize; 4] {
        std::array::from_fn(|k| self.erode_why[k].load(Ordering::Relaxed))
    }

    /// Diagnóstico: (interna, hourglass) por peça, separando elementos vivos e erodidos.
    /// Índice: [peça][0 = vivo, 1 = erodido] → (IE, HG).
    pub fn energy_by_part(&self) -> Vec<[(f64, f64); 2]> {
        let np = self.mats.len();
        let mut out = vec![[(0.0, 0.0); 2]; np];
        for (e, st) in self.elem.iter().enumerate() {
            let k = if st.alive { 0 } else { 1 };
            let o = &mut out[self.mesh.part[e] as usize][k];
            o.0 += st.e_int;
            o.1 += st.e_hg;
        }
        out
    }

    /// Velocidade do centro de massa dos nós que pertencem à peça (elementos vivos ou não).
    pub fn part_velocity(&self, part: u8) -> [f64; 3] {
        let mut owner = vec![false; self.x.len()];
        for (e, h) in self.mesh.hex.iter().enumerate() {
            if self.mesh.part[e] == part {
                for &k in h {
                    owner[k as usize] = true;
                }
            }
        }
        let (mut p, mut m) = ([0.0; 3], 0.0);
        for i in 0..self.x.len() {
            if owner[i] {
                for a in 0..3 {
                    p[a] += self.mass[i] * self.v[i][a];
                }
                m += self.mass[i];
            }
        }
        p.map(|v| v / m.max(1e-30))
    }

    /// Centro de massa dos nós da peça.
    pub fn part_center(&self, part: u8) -> [f64; 3] {
        let mut owner = vec![false; self.x.len()];
        for (e, h) in self.mesh.hex.iter().enumerate() {
            if self.mesh.part[e] == part {
                for &k in h {
                    owner[k as usize] = true;
                }
            }
        }
        let (mut p, mut m) = ([0.0; 3], 0.0);
        for i in 0..self.x.len() {
            if owner[i] {
                for a in 0..3 {
                    p[a] += self.mass[i] * self.x[i][a];
                }
                m += self.mass[i];
            }
        }
        p.map(|v| v / m.max(1e-30))
    }

    /// Tensões médias (σxx, σyy, σzz) ponderadas pelo volume de um conjunto de elementos.
    pub fn mean_stress(&self, elems: impl Iterator<Item = usize>) -> [f64; 3] {
        let (mut acc, mut vt) = ([0.0; 3], 0.0);
        for e in elems {
            let st = &self.elem[e];
            let p = st.p + st.q;
            for a in 0..3 {
                acc[a] += (st.s[a] - p) * st.vol;
            }
            vt += st.vol;
        }
        acc.map(|v| v / vt.max(1e-30))
    }

    pub fn eps_p(&self, e: usize) -> f64 {
        self.elem[e].eps_p
    }
    pub fn temp(&self, e: usize) -> f64 {
        self.elem[e].temp
    }
    /// Von Mises do elemento
    pub fn von_mises(&self, e: usize) -> f64 {
        let s = &self.elem[e].s;
        (1.5 * (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]) + 3.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5])).sqrt()
    }

    /// Quantidade de movimento total
    pub fn momentum(&self) -> [f64; 3] {
        let mut p = [0.0; 3];
        for (v, m) in self.v.iter().zip(&self.mass) {
            for a in 0..3 {
                p[a] += m * v[a];
            }
        }
        p
    }
}

/// Maior tensão principal (tração > 0) de σ = s − p I.
fn max_principal(s: &[f64; 6], p: f64) -> f64 {
    // invariantes do desviador; autovalores pela forma trigonométrica
    let j2 = 0.5 * (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]) + s[3] * s[3] + s[4] * s[4] + s[5] * s[5];
    if j2 < 1e-30 {
        return -p;
    }
    let j3 = s[0] * (s[1] * s[2] - s[4] * s[4]) - s[3] * (s[3] * s[2] - s[4] * s[5]) + s[5] * (s[3] * s[4] - s[1] * s[5]);
    let r = (j3 / 2.0 * (3.0 / j2).powf(1.5)).clamp(-1.0, 1.0);
    let th = r.acos() / 3.0;
    2.0 * (j2 / 3.0).sqrt() * th.cos() - p
}

/// Velocidade do som (bulk tangente + cisalhamento).
fn sound_speed(m: &FemMat, mu: f64, rho: f64) -> f64 {
    ((m.bulk_tangent(mu) + 4.0 / 3.0 * m.g) / rho.max(1e-9)).sqrt()
}

/// Matriz B média de Flanagan-Belytschko: B_i = (1/V) ∫ ∂N_i/∂x dV, exata para o hexaedro
/// trilinear, e o volume exato. Usa a identidade B_i = (1/V) ∂V/∂x_i.
pub fn b_matrix(x: &[[f64; 3]; 8]) -> ([[f64; 3]; 8], f64) {
    // ∂V/∂x_i pela fórmula fechada de Flanagan & Belytschko (1981), eq. (79):
    // B_i^x = 1/12 [ y_j (z_k ...) ...] — implementada com a permutação cíclica padrão.
    let y: [f64; 8] = std::array::from_fn(|k| x[k][1]);
    let z: [f64; 8] = std::array::from_fn(|k| x[k][2]);
    let xx: [f64; 8] = std::array::from_fn(|k| x[k][0]);
    let bx = fb_grad(&y, &z);
    let by = fb_grad(&z, &xx);
    let bz = fb_grad(&xx, &y);
    let vol: f64 = (0..8).map(|k| xx[k] * bx[k]).sum();
    let inv = 1.0 / vol;
    let b: [[f64; 3]; 8] = std::array::from_fn(|k| [bx[k] * inv, by[k] * inv, bz[k] * inv]);
    (b, vol)
}

/// Gradiente do volume em relação à coordenada x dos nós, dados y e z (FB 1981, eq. 79), na
/// numeração 0..7 deste módulo (= 1..8 do artigo).
fn fb_grad(y: &[f64; 8], z: &[f64; 8]) -> [f64; 8] {
    // Para cada nó i, a fórmula usa uma permutação dos outros nós. Tabela da permutação
    // (j, k, l, m, n, o?) ... implementada com a forma geral: B_1 = 1/12 [ y2((z6−z3)−(z4−z5)) + y3(z2−z4)
    // + y4((z3−z8)−(z5−z2)) + y5((z8−z6)−(z2−z4)) + y6(z5−z2) + y8(z4−z5) ]
    // e as demais por permutação de nós (Flanagan & Belytschko, Tabela 1).
    const P: [[usize; 8]; 8] = [
        [0, 1, 2, 3, 4, 5, 6, 7],
        [1, 2, 3, 0, 5, 6, 7, 4],
        [2, 3, 0, 1, 6, 7, 4, 5],
        [3, 0, 1, 2, 7, 4, 5, 6],
        [4, 7, 6, 5, 0, 3, 2, 1],
        [5, 4, 7, 6, 1, 0, 3, 2],
        [6, 5, 4, 7, 2, 1, 0, 3],
        [7, 6, 5, 4, 3, 2, 1, 0],
    ];
    let mut out = [0.0; 8];
    for i in 0..8 {
        let p = &P[i];
        let (y2, y3, y4, y5, y6, y8) = (y[p[1]], y[p[2]], y[p[3]], y[p[4]], y[p[5]], y[p[7]]);
        let (z2, z3, z4, z5, z6, z8) = (z[p[1]], z[p[2]], z[p[3]], z[p[4]], z[p[5]], z[p[7]]);
        out[i] = (y2 * ((z6 - z3) - (z4 - z5)) + y3 * (z2 - z4) + y4 * ((z3 - z8) - (z5 - z2)) + y5 * ((z8 - z6) - (z2 - z4)) + y6 * (z5 - z2) + y8 * (z4 - z5)) / 12.0;
    }
    out
}

/// Vetores de hourglass de Flanagan-Belytschko: γ_α = h_α − (h_α · x_a) B_a.
fn gamma(x: &[[f64; 3]; 8], b: &[[f64; 3]; 8]) -> [[f64; 8]; 4] {
    let h: [[f64; 8]; 4] = [
        std::array::from_fn(|k| NODE_SIGNS[k][1] * NODE_SIGNS[k][2]),
        std::array::from_fn(|k| NODE_SIGNS[k][0] * NODE_SIGNS[k][2]),
        std::array::from_fn(|k| NODE_SIGNS[k][0] * NODE_SIGNS[k][1]),
        std::array::from_fn(|k| NODE_SIGNS[k][0] * NODE_SIGNS[k][1] * NODE_SIGNS[k][2]),
    ];
    let mut g = [[0.0; 8]; 4];
    for al in 0..4 {
        let hx: [f64; 3] = std::array::from_fn(|a| (0..8).map(|k| h[al][k] * x[k][a]).sum());
        for k in 0..8 {
            g[al][k] = h[al][k] - (hx[0] * b[k][0] + hx[1] * b[k][1] + hx[2] * b[k][2]);
        }
        // normalização usual (γ/√8) para a rigidez ser independente da escala dos vetores h
        for k in 0..8 {
            g[al][k] /= 8f64.sqrt();
        }
    }
    g
}

/// Comprimento característico: volume / maior área de face.
fn char_length(x: &[[f64; 3]; 8], vol: f64) -> f64 {
    let mut amax: f64 = 0.0;
    for f in &mesh::FACES {
        let (a, b, c, d) = (x[f[0]], x[f[1]], x[f[2]], x[f[3]]);
        let d1 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let d2 = [d[0] - b[0], d[1] - b[1], d[2] - b[2]];
        let cr = [d1[1] * d2[2] - d1[2] * d2[1], d1[2] * d2[0] - d1[0] * d2[2], d1[0] * d2[1] - d1[1] * d2[0]];
        amax = amax.max(0.5 * (cr[0] * cr[0] + cr[1] * cr[1] + cr[2] * cr[2]).sqrt());
    }
    vol / amax.max(1e-30)
}

/// Rotação de Jaumann da tensão desviadora: s += (W s − s W) dt, com W antissimétrico
/// dado por (w_xy, w_yz, w_zx) onde W_xy = w_xy.
fn jaumann(s: [f64; 6], w: [f64; 3], dt: f64) -> [f64; 6] {
    let sm = [[s[0], s[3], s[5]], [s[3], s[1], s[4]], [s[5], s[4], s[2]]];
    let wm = [[0.0, w[0], -w[2]], [-w[0], 0.0, w[1]], [w[2], -w[1], 0.0]];
    let mut r = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            let mut acc = 0.0;
            for k in 0..3 {
                acc += wm[i][k] * sm[k][j] - sm[i][k] * wm[k][j];
            }
            r[i][j] = sm[i][j] + acc * dt;
        }
    }
    [r[0][0], r[1][1], r[2][2], 0.5 * (r[0][1] + r[1][0]), 0.5 * (r[1][2] + r[2][1]), 0.5 * (r[2][0] + r[0][2])]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube(l: f64) -> [[f64; 3]; 8] {
        std::array::from_fn(|k| [NODE_SIGNS[k][0] * l / 2.0, NODE_SIGNS[k][1] * l / 2.0, NODE_SIGNS[k][2] * l / 2.0])
    }

    #[test]
    fn b_matrix_cube() {
        let x = cube(2.0);
        let (b, v) = b_matrix(&x);
        assert!((v - 8.0).abs() < 1e-12, "volume {v}");
        // cubo de lado 2 (x = ξ): ∂N_i/∂x = ξ_i / 8
        for k in 0..8 {
            for a in 0..3 {
                assert!((b[k][a] - NODE_SIGNS[k][a] / 8.0).abs() < 1e-12);
            }
        }
    }

    #[test]
    fn b_matrix_reproduces_linear_field_on_distorted_hex() {
        let mut x = cube(1.0);
        x[6] = [0.7, 0.55, 0.62];
        x[1][2] -= 0.1;
        let (b, v) = b_matrix(&x);
        assert!((v - mesh::hex_volume(&x)).abs() < 1e-12 * v.abs().max(1.0) * 10.0, "{v} vs {}", mesh::hex_volume(&x));
        // campo linear u = G x → gradiente exato
        let g = [[0.3, -0.2, 0.1], [0.05, 0.4, -0.3], [0.2, 0.1, -0.5]];
        for a in 0..3 {
            for c in 0..3 {
                let s: f64 = (0..8).map(|k| (0..3).map(|d| g[a][d] * x[k][d]).sum::<f64>() * b[k][c]).sum();
                assert!((s - g[a][c]).abs() < 1e-12, "{a}{c}: {s}");
            }
        }
        // hourglass ortogonal a campos lineares
        let gm = gamma(&x, &b);
        for al in 0..4 {
            for d in 0..3 {
                let s: f64 = (0..8).map(|k| gm[al][k] * x[k][d]).sum();
                assert!(s.abs() < 1e-12);
            }
            let s: f64 = gm[al].iter().sum();
            assert!(s.abs() < 1e-12);
        }
    }

    #[test]
    fn jaumann_rotation_keeps_invariants() {
        let s = [100.0, -40.0, -60.0, 30.0, 0.0, 10.0];
        let r = jaumann(s, [0.3, -0.1, 0.2], 1e-3);
        let j2 = |s: [f64; 6]| 0.5 * (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]) + s[3] * s[3] + s[4] * s[4] + s[5] * s[5];
        assert!((j2(r) - j2(s)).abs() / j2(s) < 1e-5);
        assert!((r[0] + r[1] + r[2]).abs() < 1e-9);
    }
}
