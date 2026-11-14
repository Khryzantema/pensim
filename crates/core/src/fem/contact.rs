//! Contato nó-contra-superfície por penalidade, com atrito de Coulomb (fase 2).
//!
//! Contato geral: todos os nós de superfície (e as partículas, nós sem elemento vivo) contra
//! todas as faces externas, inclusive do mesmo corpo (autocontato: o tampão contra a parede do
//! furo). Um par é ignorado se o nó pertence a um elemento vivo que toca a face.
//! Rigidez: k = s·K·A²/V da face (forma padrão dos códigos explícitos).
use super::mesh::FACES;
use rayon::prelude::*;
use std::collections::HashMap;

/// Fração da rigidez do elemento usada na penalidade.
pub const PENALTY: f64 = 0.1;
/// Rigidez mínima pela massa do nó e o passo (tipo SOFT=1 do LS-DYNA): k ≥ SOFT·m/dt²
pub const SOFT: f64 = 0.1;
/// Amortecimento viscoso no contato (fração do crítico). 0,3 calibrado nos 80 casos esfera×chapa
/// contra o OpenCourant (docs/NOVO-NUCLEO.md, seção 21); PENSIM_CDAMP muda.
pub const CDAMP: f64 = 0.3;

fn cdamp() -> f64 {
    static K: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *K.get_or_init(|| std::env::var("PENSIM_CDAMP").ok().and_then(|v| v.parse().ok()).unwrap_or(CDAMP))
}
fn soft() -> f64 {
    static K: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
    *K.get_or_init(|| std::env::var("PENSIM_SOFT").ok().and_then(|v| v.parse().ok()).unwrap_or(SOFT))
}

/// Face externa: elemento dono e índice local da face.
#[derive(Clone, Copy, Debug)]
pub struct SurfFace {
    pub elem: u32,
    pub face: u8,
}

/// Uma força de contato: no nó escravo `node` e, com sinal oposto e pesos `w`, nos 4 nós da face.
pub struct Hit {
    pub node: u32,
    pub fnodes: [u32; 4],
    pub w: [f64; 4],
    pub f: [f64; 3],
    /// penetração tratada como inicial (não gera força); mantida no passo seguinte
    pub offset: f64,
}

pub struct ContactInput<'a> {
    pub x: &'a [[f64; 3]],
    pub v: &'a [[f64; 3]],
    pub mass: &'a [f64],
    pub hex: &'a [[u32; 8]],
    pub alive: &'a [bool],
    /// K do material da face (por elemento) e volume atual
    pub bulk: &'a [f64],
    pub vol: &'a [f64],
    pub lc: &'a [f64],
    pub surf: &'a [SurfFace],
    /// nós candidatos (superfície + partículas)
    pub nodes: &'a [u32],
    /// nó → elementos vivos (CSR de elemento·8 + local; os mortos são filtrados aqui)
    pub adj_ptr: &'a [u32],
    pub adj: &'a [u32],
    pub friction: f64,
    pub dt: f64,
    /// penetração inicial guardada de cada nó no passo anterior (NaN = sem contato)
    pub prev_offset: &'a [f64],
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Pesos baricêntricos de p no triângulo (a, b, c) (p no plano).
fn bary(p: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let (v0, v1, v2) = (sub(b, a), sub(c, a), sub(p, a));
    let (d00, d01, d11, d20, d21) = (dot(v0, v0), dot(v0, v1), dot(v1, v1), dot(v2, v0), dot(v2, v1));
    let den = (d00 * d11 - d01 * d01).max(1e-300);
    let v = (d11 * d20 - d01 * d21) / den;
    let w = (d00 * d21 - d01 * d20) / den;
    [1.0 - v - w, v, w]
}

/// Calcula as forças de contato do instante atual.
pub fn forces(c: &ContactInput) -> Vec<Hit> {
    if c.surf.is_empty() || c.nodes.is_empty() {
        return Vec::new();
    }
    // geometria das faces
    struct F {
        n: [u32; 4],
        p: [[f64; 3]; 4],
        cen: [f64; 3],
        nrm: [f64; 3],
        k: f64,
        depth: f64,
        r: f64,
    }
    let faces: Vec<F> = c
        .surf
        .par_iter()
        .map(|sf| {
            let h = &c.hex[sf.elem as usize];
            let n = FACES[sf.face as usize].map(|l| h[l]);
            let p = n.map(|k| c.x[k as usize]);
            let cen = [0, 1, 2].map(|a| 0.25 * (p[0][a] + p[1][a] + p[2][a] + p[3][a]));
            let cr = cross(sub(p[2], p[0]), sub(p[3], p[1]));
            let a2 = dot(cr, cr).sqrt();
            let area = 0.5 * a2;
            let mut nrm = if a2 > 0.0 { cr.map(|v| v / a2) } else { [0.0; 3] };
            // normal para fora do elemento (pelo centróide do elemento)
            let ec = [0, 1, 2].map(|a| h.iter().map(|&k| c.x[k as usize][a]).sum::<f64>() / 8.0);
            if dot(sub(cen, ec), nrm) < 0.0 {
                nrm = nrm.map(|v| -v);
            }
            let e = sf.elem as usize;
            let k = PENALTY * c.bulk[e] * area * area / c.vol[e].max(1e-30);
            let r = p.iter().map(|q| dot(sub(*q, cen), sub(*q, cen)).sqrt()).fold(0.0, f64::max);
            F { n, p, cen, nrm, k, depth: 0.5 * c.lc[e], r }
        })
        .collect();
    // grade espacial pelas faces
    let cell = faces.iter().map(|f| 2.0 * f.r.max(f.depth)).fold(0.0, f64::max).max(1e-9);
    let key = |p: [f64; 3]| p.map(|v| (v / cell).floor() as i64);
    let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::with_capacity(faces.len());
    for (i, f) in faces.iter().enumerate() {
        grid.entry(key(f.cen)).or_default().push(i as u32);
    }
    // elementos vivos de um nó
    let node_elems = |k: u32| -> smallvec_like::Small {
        let mut s = smallvec_like::Small::new();
        for &a in &c.adj[c.adj_ptr[k as usize] as usize..c.adj_ptr[k as usize + 1] as usize] {
            let e = a / 8;
            if c.alive[e as usize] {
                s.push(e);
            }
        }
        s
    };
    c.nodes
        .par_iter()
        .filter_map(|&nd| {
            let xp = c.x[nd as usize];
            let my = node_elems(nd);
            // partícula: nó sem elemento vivo (sobra de erosão)
            let particle = my.iter().next().is_none();
            let kc = key(xp);
            let mut best: Option<(f64, usize, [f64; 4])> = None;
            for dxk in -1..=1 {
                for dyk in -1..=1 {
                    for dzk in -1..=1 {
                        let Some(list) = grid.get(&[kc[0] + dxk, kc[1] + dyk, kc[2] + dzk]) else { continue };
                        for &fi in list {
                            let f = &faces[fi as usize];
                            if f.n.contains(&nd) {
                                continue;
                            }
                            let d = dot(sub(xp, f.cen), f.nrm);
                            // partícula é procurada mais fundo (até um elemento inteiro): ela pode ter
                            // nascido dentro do outro corpo quando o elemento dela foi apagado
                            let depth = f.depth;
                            if d >= 0.0 || d < -depth {
                                continue;
                            }
                            // vizinho topológico: um elemento vivo do nó contém um nó da face
                            if my.iter().any(|&e| f.n.iter().any(|q| c.hex[e as usize].contains(q))) {
                                continue;
                            }
                            let pr = sub(xp, f.nrm.map(|v| v * d));
                            // dentro do quadrilátero?
                            let mut inside = true;
                            for k in 0..4 {
                                let e = sub(f.p[(k + 1) % 4], f.p[k]);
                                if dot(cross(e, sub(pr, f.p[k])), f.nrm) < -1e-12 * dot(e, e) {
                                    inside = false;
                                    break;
                                }
                            }
                            if !inside {
                                continue;
                            }
                            if best.map_or(true, |b| -d < b.0) {
                                // pesos: triângulo (0,1,2) ou (0,2,3)
                                let b1 = bary(pr, f.p[0], f.p[1], f.p[2]);
                                let w = if b1.iter().all(|&v| v >= -1e-9) {
                                    [b1[0], b1[1], b1[2], 0.0]
                                } else {
                                    let b2 = bary(pr, f.p[0], f.p[2], f.p[3]);
                                    [b2[0], 0.0, b2[1], b2[2]]
                                };
                                best = Some((-d, fi as usize, w));
                            }
                        }
                    }
                }
            }
            let (pen, fi, w) = best?;
            let f = &faces[fi];
            // penetração que não pode ter surgido neste passo (face recém-exposta pela erosão,
            // nó que já estava dentro) vira referência: só o que passar dela gera força, e a
            // referência só diminui. Assim o contato não injeta energia.
            let vf0 = [0, 1, 2].map(|a| (0..4).map(|k| w[k] * c.v[f.n[k] as usize][a]).sum::<f64>());
            let vn0 = dot(sub(c.v[nd as usize], vf0), f.nrm);
            let prev = c.prev_offset[nd as usize];
            // partícula: a penetração com que ela nasceu é descontada aos poucos (5% por passo),
            // então ela sai de dentro do outro corpo devagar, sem esmagar a borda da cratera
            let offset = if prev.is_nan() {
                let reach = 2.0 * vn0.abs() * c.dt + 1e-3 * f.depth;
                (pen - reach).max(0.0)
            } else if particle {
                (prev * 0.95).min(pen)
            } else {
                prev.min(pen)
            };
            let eff = pen - offset;
            let mn = c.mass[nd as usize];
            // massa efetiva do par nó × face (a reação vai para os nós da face, que podem ser leves)
            let mf: f64 = (0..4).map(|k| w[k] * c.mass[f.n[k] as usize]).sum::<f64>().max(1e-30);
            let meff = mn * mf / (mn + mf);
            let k = f.k.max(soft() * mn / (c.dt * c.dt)).min(if particle { 0.25 * meff / (c.dt * c.dt) } else { f64::INFINITY });
            let mut fnm = k * eff;
            // amortecimento viscoso do contato (fração ζ do crítico do par nó × face)
            let zeta = cdamp();
            if zeta > 0.0 && eff > 0.0 {
                let mf: f64 = (0..4).map(|kk| w[kk] * c.mass[f.n[kk] as usize]).sum::<f64>().max(1e-30);
                let meff = mn * mf / (mn + mf);
                fnm = (fnm - 2.0 * zeta * (k * meff).sqrt() * vn0).max(0.0);
            }
            let mut force = f.nrm.map(|v| v * fnm);
            // atrito de Coulomb, limitado a não inverter a velocidade relativa no passo
            if c.friction > 0.0 {
                let vf = [0, 1, 2].map(|a| (0..4).map(|k| w[k] * c.v[f.n[k] as usize][a]).sum::<f64>());
                let vr = sub(c.v[nd as usize], vf);
                let vn = dot(vr, f.nrm);
                let vt = sub(vr, f.nrm.map(|v| v * vn));
                let vtl = dot(vt, vt).sqrt();
                if vtl > 1e-12 {
                    let stop = 0.5 * c.mass[nd as usize] * vtl / c.dt;
                    let ft = (c.friction * fnm).min(stop);
                    for a in 0..3 {
                        force[a] -= ft * vt[a] / vtl;
                    }
                }
            }
            Some(Hit { node: nd, fnodes: f.n, w, f: force, offset })
        })
        .collect()
}

/// Vetor pequeno sem alocação para os ≤ 8 elementos de um nó.
mod smallvec_like {
    pub struct Small {
        v: [u32; 8],
        n: usize,
    }
    impl Small {
        pub fn new() -> Self {
            Small { v: [0; 8], n: 0 }
        }
        pub fn push(&mut self, e: u32) {
            if self.n < 8 {
                self.v[self.n] = e;
                self.n += 1;
            }
        }
        pub fn iter(&self) -> impl Iterator<Item = &u32> {
            self.v[..self.n].iter()
        }
    }
}
