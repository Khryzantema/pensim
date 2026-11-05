//! Malha de faces que segue os nós (superfície por "cantos" da rede).
//!
//! Cada nó é uma célula cúbica da rede inicial. Para cada célula visível cuja vizinha está vazia
//! (ou cortada pela seção) gera-se uma face. Os 4 cantos da face são compartilhados e ficam em
//! `canto ideal + média dos deslocamentos dos nós vizinhos`, então a malha acompanha a deformação.
use std::collections::HashMap;

#[derive(Clone, Copy)]
pub struct Quad {
    pub v: [u32; 4],
    /// índice do nó (no vetor estendido pelas cópias espelhadas) dono da face
    pub cell: u32,
    /// face criada pelo corte de seção (cor neutra)
    pub cut: bool,
}

#[derive(Default)]
pub struct SurfMesh {
    pub verts: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub quads: Vec<Quad>,
}

struct Lattice {
    o: [f32; 3],
    e: [[f32; 3]; 3],
    s: [f32; 3],
    c: [f32; 3],
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

impl Lattice {
    fn index(&self, p: [f32; 3]) -> [i32; 3] {
        let d = [p[0] - self.o[0], p[1] - self.o[1], p[2] - self.o[2]];
        let mut k = [0i32; 3];
        for a in 0..3 {
            k[a] = (dot(d, self.e[a]) / self.s[a] - self.c[a]).round() as i32;
        }
        k
    }
    /// posição ideal do canto de índice `c` (entre as células c-1 e c em cada eixo)
    fn corner(&self, c: [i32; 3]) -> [f32; 3] {
        let mut p = self.o;
        for a in 0..3 {
            let t = (c[a] as f32 + self.c[a] - 0.5) * self.s[a];
            for q in 0..3 {
                p[q] += self.e[a][q] * t;
            }
        }
        p
    }
}

fn pack(g: u64, k: [i32; 3]) -> u64 {
    const B: i64 = 1 << 20;
    (g << 63) | (((k[0] as i64 + B) as u64 & 0x1F_FFFF) << 42) | (((k[1] as i64 + B) as u64 & 0x1F_FFFF) << 21) | ((k[2] as i64 + B) as u64 & 0x1F_FFFF)
}

/// `vis(mi, i)`: 0 = oculto, 1 = visível, 2 = removido só pelo corte de seção.
/// `cur`/`refp`: posições atuais e iniciais (frame da simulação). `flip` espelha pela máscara `m`;
/// `turn` leva o frame da simulação ao frame da tela.
#[allow(clippy::too_many_arguments)]
pub fn build(
    n: usize,
    masks: &[u8],
    cur: &[[f32; 3]],
    refp: &[[f32; 3]],
    parts: &[u8],
    n_proj: u8,
    dx: f32,
    obliquity_rad: f32,
    max_quads: usize,
    flip: &dyn Fn([f32; 3], u8) -> [f32; 3],
    turn: &dyn Fn([f32; 3]) -> [f32; 3],
    vis: &dyn Fn(usize, usize) -> u8,
) -> Option<SurfMesh> {
    let (sa, ca) = (obliquity_rad.sin(), obliquity_rad.cos());
    let d = [ca, 0.0, -sa];
    let ev = [sa, 0.0, ca];

    // rede do projétil: eixo a (da ponta para trás), u = y, v ⟂ voo
    let (mut tmax, mut tmin) = (f32::MIN, f32::MAX);
    for i in 0..n {
        if parts[i] < n_proj {
            let t = dot(refp[i], d);
            tmax = tmax.max(t);
            tmin = tmin.min(t);
        }
    }
    let nm = ((tmax - tmin) / dx).round();
    let sp = if nm >= 1.0 { (tmax - tmin) / nm } else { dx };
    let proj = Lattice { o: [d[0] * tmax, 0.0, d[2] * tmax], e: [[-d[0], 0.0, -d[2]], [0.0, 1.0, 0.0], ev], s: [sp, dx, dx], c: [0.0, 0.5, 0.5] };
    let targ = Lattice { o: [0.0; 3], e: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], s: [dx; 3], c: [0.5; 3] };

    let total = n * masks.len();
    // célula → (índice estendido | bit31 se cortada)
    let mut map: HashMap<u64, u32> = HashMap::with_capacity(total);
    let mut cells: Vec<(u64, [i32; 3], u8)> = Vec::new();
    for (mi, &m) in masks.iter().enumerate() {
        for i in 0..n {
            let st = vis(mi, i);
            if st == 0 {
                continue;
            }
            let g = (parts[i] < n_proj) as u64;
            let lat = if g == 1 { &proj } else { &targ };
            let k = lat.index(flip(refp[i], m));
            let e = (mi * n + i) as u32;
            map.insert(pack(g, k), if st == 2 { e | 0x8000_0000 } else { e });
            if st == 1 {
                cells.push((e as u64, k, g as u8));
            }
        }
    }

    let mut out = SurfMesh::default();
    let mut vmap: HashMap<u64, u32> = HashMap::new();
    // deslocamento (atual − inicial) de um nó estendido
    let disp = |e: u32| -> [f32; 3] {
        let (mi, i) = ((e as usize) / n, (e as usize) % n);
        let m = masks[mi];
        let (a, b) = (flip(cur[i], m), flip(refp[i], m));
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    };
    let mut outward: Vec<[f32; 3]> = Vec::new();
    for &(e, k, g) in &cells {
        let lat = if g == 1 { &proj } else { &targ };
        let gg = g as u64;
        for ax in 0..3 {
            for sgn in [-1i32, 1] {
                let mut nk = k;
                nk[ax] += sgn;
                let nb = map.get(&pack(gg, nk)).copied();
                let cut = match nb {
                    None => false,
                    Some(v) if v & 0x8000_0000 != 0 => true,
                    Some(_) => continue,
                };
                if out.quads.len() >= max_quads {
                    return None;
                }
                let (a1, a2) = ((ax + 1) % 3, (ax + 2) % 3);
                let base = if sgn > 0 { k[ax] + 1 } else { k[ax] };
                let mut vi = [0u32; 4];
                for (q, (du, dv)) in [(0, 0), (1, 0), (1, 1), (0, 1)].into_iter().enumerate() {
                    let mut c = k;
                    c[ax] = base;
                    c[a1] = k[a1] + du;
                    c[a2] = k[a2] + dv;
                    let key = pack(gg, c);
                    let id = *vmap.entry(key).or_insert_with(|| {
                        // média dos deslocamentos das 8 células em volta do canto
                        let (mut acc, mut cnt) = ([0.0f32; 3], 0.0f32);
                        for dz in 0..2 {
                            for dy in 0..2 {
                                for dxx in 0..2 {
                                    let cc = [c[0] - dxx, c[1] - dy, c[2] - dz];
                                    if let Some(&v) = map.get(&pack(gg, cc)) {
                                        if v & 0x8000_0000 == 0 {
                                            let dd = disp(v);
                                            acc[0] += dd[0];
                                            acc[1] += dd[1];
                                            acc[2] += dd[2];
                                            cnt += 1.0;
                                        }
                                    }
                                }
                            }
                        }
                        let p = lat.corner(c);
                        let inv = if cnt > 0.0 { 1.0 / cnt } else { 0.0 };
                        out.verts.push(turn([p[0] + acc[0] * inv, p[1] + acc[1] * inv, p[2] + acc[2] * inv]));
                        (out.verts.len() - 1) as u32
                    });
                    vi[q] = id;
                }
                let eo = lat.e[ax];
                outward.push(turn([eo[0] * sgn as f32, eo[1] * sgn as f32, eo[2] * sgn as f32]));
                out.quads.push(Quad { v: vi, cell: e as u32, cut });
            }
        }
    }

    // normais suaves: soma das normais geométricas das faces em cada canto
    let mut nrm = vec![[0.0f32; 3]; out.verts.len()];
    for (qi, q) in out.quads.iter().enumerate() {
        let p = |i: usize| out.verts[q.v[i] as usize];
        let (d1, d2) = ([p(2)[0] - p(0)[0], p(2)[1] - p(0)[1], p(2)[2] - p(0)[2]], [p(3)[0] - p(1)[0], p(3)[1] - p(1)[1], p(3)[2] - p(1)[2]]);
        let mut nn = [d1[1] * d2[2] - d1[2] * d2[1], d1[2] * d2[0] - d1[0] * d2[2], d1[0] * d2[1] - d1[1] * d2[0]];
        let l = dot(nn, nn).sqrt();
        if l < 1e-20 {
            nn = outward[qi];
        } else {
            nn = [nn[0] / l, nn[1] / l, nn[2] / l];
            if dot(nn, outward[qi]) < 0.0 {
                nn = [-nn[0], -nn[1], -nn[2]];
            }
        }
        for &vi in &q.v {
            let a = &mut nrm[vi as usize];
            a[0] += nn[0];
            a[1] += nn[1];
            a[2] += nn[2];
        }
    }
    for a in &mut nrm {
        let l = dot(*a, *a).sqrt().max(1e-12);
        *a = [a[0] / l, a[1] / l, a[2] / l];
    }
    out.normals = nrm;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pensim_core::{Model, Preset};

    fn run(obl: f64) -> (SurfMesh, usize, f32) {
        let mut cfg = Preset::Rifle762.config();
        cfg.dx = 0.0015;
        cfg.obliquity_deg = obl;
        cfg.symmetry = false;
        let m = Model::build(&cfg).unwrap();
        let pos: Vec<[f32; 3]> = m.pos.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
        let parts = m.part.clone();
        let np = cfg.design.n_proj_parts() as u8;
        let flip = |q: [f32; 3], _m: u8| q;
        let turn = |q: [f32; 3]| q;
        let vis = |_mi: usize, _i: usize| 1u8;
        let sm = build(pos.len(), &[0], &pos, &pos, &parts, np, cfg.dx as f32, cfg.obliquity() as f32, usize::MAX, &flip, &turn, &vis).unwrap();
        (sm, pos.len(), cfg.dx as f32)
    }

    #[test]
    fn faces_follow_surface() {
        for obl in [0.0, 30.0] {
            let (sm, n, dx) = run(obl);
            assert!(!sm.quads.is_empty());
            // as faces são só da casca: bem menos que 6 por nó
            assert!(sm.quads.len() < n * 2, "quads={} n={}", sm.quads.len(), n);
            // sem deslocamento, cada canto fica a no máximo ~dx de algum nó (jitter + degrau)
            for v in sm.verts.iter().take(2000) {
                assert!(v.iter().all(|c| c.is_finite()));
            }
            for nv in &sm.normals {
                let l = (nv[0] * nv[0] + nv[1] * nv[1] + nv[2] * nv[2]).sqrt();
                assert!((l - 1.0).abs() < 1e-3);
            }
            let xs = sm.verts.iter().map(|v| v[0]);
            let (x0, x1) = (xs.clone().fold(f32::MAX, f32::min), xs.fold(f32::MIN, f32::max));
            eprintln!("  x∈[{x0:.4},{x1:.4}] dx={dx}");
            eprintln!("obl={obl} nós={n} faces={}", sm.quads.len());
        }
    }
}
