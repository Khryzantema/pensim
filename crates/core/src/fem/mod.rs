//! Núcleo novo: elementos finitos explícitos Lagrangianos (projeto em `docs/NOVO-NUCLEO.md`).
//!
//! Fase 1: malha de hexaedros, elemento de 1 ponto com hourglass por rigidez
//! (Flanagan & Belytschko 1981), taxa de Jaumann, Johnson-Cook com retorno radial, EOS
//! Mie-Grüneisen, viscosidade artificial volumétrica, diferenças centrais, parede rígida e
//! condições de simetria. Ainda sem contato entre corpos e sem erosão.
//!
//! Estado em `f64` nesta fase (correção primeiro); a troca para `f32` é da fase 6.
pub mod material;
pub mod mesh;

use material::FemMat;
use mesh::{HexMesh, NODE_SIGNS};
use rayon::prelude::*;

/// Coeficientes de viscosidade artificial (padrão dos códigos explícitos): q = ρ l (Q1 l ε̇v² − Q2 c ε̇v)
const Q1: f64 = 1.5;
const Q2: f64 = 0.06;
/// Rigidez do hourglass (fração da rigidez do elemento)
const HG_KAPPA: f64 = 0.1;
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
    /// dano de Johnson-Cook acumulado (usado a partir da fase 2)
    #[allow(dead_code)]
    dmg: f64,
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
    pub plastic: f64,
}

impl Energy {
    /// Erro de balanço: (cinética + interna + hourglass − externa) − total inicial.
    pub fn balance(&self, e0: f64) -> f64 {
        self.kinetic + self.internal + self.hourglass - self.external - e0
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
}

impl Fem {
    pub fn new(model: FemModel) -> Fem {
        let FemModel { mesh, mats, v0, prescribed, walls } = model;
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
            elem[e] = Elem { v0: vol, vol, mass: em, temp: m.t_room, ..Default::default() };
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
        };
        s.apply_bc_velocity(0.0);
        // forças e passo iniciais (tensão zero; só fixa lc, c e dt)
        s.element_pass(0.0, true);
        s.assemble();
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
        let m = self.elem.par_iter().map(|e| e.lc / e.c.max(1e-9)).reduce(|| f64::INFINITY, f64::min);
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
        self.assemble();
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
        self.elem.par_iter_mut().zip(self.ef.par_iter_mut()).enumerate().for_each(|(e, (st, fe))| {
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
            let k_hg = HG_KAPPA * (m.bulk_tangent(mu.max(0.0)) + 4.0 / 3.0 * m.g) * bb * vol / 8.0;
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

    // ----------------------------------------------------------------- consultas

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
