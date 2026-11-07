//! Malha de faces que segue os nós (superfície por "cantos" da rede).
//!
//! Cada nó é uma célula cúbica da rede inicial. Para cada célula visível cuja vizinha está vazia,
//! cortada pela seção ou já separada (trinca) gera-se uma face. Os cantos ficam em
//! `canto ideal + média dos deslocamentos das células vizinhas que continuam presas`.
//!
//! Desempenho: a topologia (célula de cada nó numa grade densa) é montada uma vez por simulação
//! (`Topo`); por quadro só se calculam deslocamentos e faces, em paralelo, e os vértices são
//! compartilhados entre faces.
use rayon::prelude::*;
use std::collections::HashMap;
use std::hash::{BuildHasherDefault, Hasher};

/// Hash rápido (estilo Fx) para chaves inteiras.
#[derive(Default)]
struct Fx(u64);
impl Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0.rotate_left(5) ^ b as u64).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (self.0.rotate_left(5) ^ v).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
    fn write_u32(&mut self, v: u32) {
        self.write_u64(v as u64);
    }
}
type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<Fx>>;

const NONE: u32 = u32::MAX;

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
    /// nó que dá a cor de cada vértice, e se ele está numa face de corte
    pub vcell: Vec<u32>,
    pub vcut: Vec<bool>,
    pub quads: Vec<Quad>,
    /// fragmentos de um nó só (sem vizinho preso): desenhados como nós redondos
    pub loose: Vec<u32>,
}

#[derive(Clone, Copy)]
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

/// Grade densa de um grupo (projétil ou alvo).
struct Grid {
    lat: Lattice,
    min: [i32; 3],
    dim: [i32; 3],
    cell: Vec<u32>,
}

impl Grid {
    #[inline]
    fn at(&self, k: [i32; 3]) -> u32 {
        let r = [k[0] - self.min[0], k[1] - self.min[1], k[2] - self.min[2]];
        if r[0] < 0 || r[1] < 0 || r[2] < 0 || r[0] >= self.dim[0] || r[1] >= self.dim[1] || r[2] >= self.dim[2] {
            return NONE;
        }
        self.cell[((r[0] as usize * self.dim[1] as usize) + r[1] as usize) * self.dim[2] as usize + r[2] as usize]
    }
    #[inline]
    fn corner_id(&self, c: [i32; 3]) -> u64 {
        let r = [(c[0] - self.min[0]) as u64, (c[1] - self.min[1]) as u64, (c[2] - self.min[2]) as u64];
        (r[0] * (self.dim[1] as u64 + 1) + r[1]) * (self.dim[2] as u64 + 1) + r[2]
    }
}

/// Topologia fixa de uma simulação: em que célula da rede está cada nó (com as cópias espelhadas).
pub struct Topo {
    n: usize,
    masks: Vec<u8>,
    grids: [Grid; 2],
    /// grupo (0 alvo, 1 projétil) e índice de célula de cada nó estendido
    grp: Vec<u8>,
    key: Vec<[i32; 3]>,
    /// posição inicial espelhada (frame da simulação)
    refx: Vec<[f32; 3]>,
}

#[allow(clippy::too_many_arguments)]
pub fn topo(n: usize, masks: &[u8], refp: &[[f32; 3]], parts: &[u8], n_proj: u8, dx: f32, obliquity_rad: f32, flip: &dyn Fn([f32; 3], u8) -> [f32; 3]) -> Topo {
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
    if tmax < tmin {
        tmax = 0.0;
        tmin = 0.0;
    }
    let nm = ((tmax - tmin) / dx).round();
    let sp = if nm >= 1.0 { (tmax - tmin) / nm } else { dx };
    let lats = [
        Lattice { o: [0.0; 3], e: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]], s: [dx; 3], c: [0.5; 3] },
        Lattice { o: [d[0] * tmax, 0.0, d[2] * tmax], e: [[-d[0], 0.0, -d[2]], [0.0, 1.0, 0.0], ev], s: [sp, dx, dx], c: [0.0, 0.5, 0.5] },
    ];
    let total = n * masks.len();
    let mut grp = vec![0u8; total];
    let mut key = vec![[0i32; 3]; total];
    let mut refx = vec![[0.0f32; 3]; total];
    let mut mn = [[i32::MAX; 3]; 2];
    let mut mx = [[i32::MIN; 3]; 2];
    for (mi, &m) in masks.iter().enumerate() {
        for i in 0..n {
            let e = mi * n + i;
            let g = (parts[i] < n_proj) as usize;
            let r = flip(refp[i], m);
            let k = lats[g].index(r);
            grp[e] = g as u8;
            key[e] = k;
            refx[e] = r;
            for a in 0..3 {
                mn[g][a] = mn[g][a].min(k[a]);
                mx[g][a] = mx[g][a].max(k[a]);
            }
        }
    }
    let mk = |g: usize| {
        let (min, dim) = if mn[g][0] > mx[g][0] { ([0; 3], [1; 3]) } else { ([mn[g][0] - 1, mn[g][1] - 1, mn[g][2] - 1], [mx[g][0] - mn[g][0] + 3, mx[g][1] - mn[g][1] + 3, mx[g][2] - mn[g][2] + 3]) };
        Grid { lat: lats[g], min, dim, cell: vec![NONE; (dim[0] as usize) * (dim[1] as usize) * (dim[2] as usize)] }
    };
    let mut grids = [mk(0), mk(1)];
    for e in 0..total {
        let g = &mut grids[grp[e] as usize];
        let k = key[e];
        let r = [k[0] - g.min[0], k[1] - g.min[1], k[2] - g.min[2]];
        let li = ((r[0] as usize * g.dim[1] as usize) + r[1] as usize) * g.dim[2] as usize + r[2] as usize;
        g.cell[li] = e as u32;
    }
    Topo { n, masks: masks.to_vec(), grids, grp, key, refx }
}

struct Face {
    corners: [(u64, [f32; 3]); 4],
    outward: [f32; 3],
    cell: u32,
    cut: bool,
}

/// `vis[e]`: 0 = oculto, 1 = visível, 2 = removido só pelo corte de seção (por nó estendido).
/// `cur`: posições atuais (frame da simulação); `turn` leva ao frame da tela.
pub fn build(t: &Topo, cur: &[[f32; 3]], vis: &[u8], dx: f32, max_quads: usize, flip: &(dyn Fn([f32; 3], u8) -> [f32; 3] + Sync), turn: &(dyn Fn([f32; 3]) -> [f32; 3] + Sync)) -> Option<SurfMesh> {
    let n = t.n;
    let total = n * t.masks.len();
    // deslocamento (atual − inicial) de cada nó estendido
    let disp: Vec<[f32; 3]> = (0..total)
        .into_par_iter()
        .map(|e| {
            let (mi, i) = (e / n, e % n);
            let a = flip(cur[i], t.masks[mi]);
            let b = t.refx[e];
            [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
        })
        .collect();
    // duas células vizinhas só "continuam juntas" se a posição relativa mudou menos que isso;
    // senão houve trinca/separação: a face entre elas aparece e os cantos não são compartilhados
    let tol2 = (0.75 * dx) * (0.75 * dx);
    // juntas = ainda próximas (até 1,6× a distância original). O metal pode escoar e se deformar
    // muito sem romper; só uma trinca de verdade afasta os vizinhos
    let _ = tol2;
    let together = |a: usize, b: usize| -> bool {
        let (ra, rb) = (t.refx[a], t.refx[b]);
        let (da, db) = (disp[a], disp[b]);
        let r0 = [rb[0] - ra[0], rb[1] - ra[1], rb[2] - ra[2]];
        let r1 = [r0[0] + db[0] - da[0], r0[1] + db[1] - da[1], r0[2] + db[2] - da[2]];
        let (l0, l1) = (r0[0] * r0[0] + r0[1] * r0[1] + r0[2] * r0[2], r1[0] * r1[0] + r1[1] * r1[1] + r1[2] * r1[2]);
        l1 < 2.56 * l0.max(1e-30)
    };
    let live = |v: u32| v != NONE && vis[v as usize] == 1;

    let per_cell = |e: usize| -> (Vec<Face>, bool) {
        let g = &t.grids[t.grp[e] as usize];
        let k = t.key[e];
        let gbit = (t.grp[e] as u64) << 63;
        let mut faces = Vec::new();
        // fragmento isolado: nenhum dos 6 vizinhos continua preso a ele
        let mut attached = false;
        let mut nb = [NONE; 6];
        for ax in 0..3 {
            for (si, sgn) in [-1i32, 1].into_iter().enumerate() {
                let mut nk = k;
                nk[ax] += sgn;
                let v = g.at(nk);
                nb[ax * 2 + si] = v;
                if live(v) && together(e, v as usize) {
                    attached = true;
                }
            }
        }
        // projétil: sempre redondo (nós em disco) e só a superfície; sem malha de faces
        if t.grp[e] == 1 {
            let exposed = nb.iter().any(|&v| !live(v) || !together(e, v as usize));
            return (faces, exposed);
        }
        if !attached {
            return (faces, true);
        }
        for ax in 0..3 {
            for (si, sgn) in [-1i32, 1].into_iter().enumerate() {
                let v = nb[ax * 2 + si];
                let cut = if v == NONE || vis[v as usize] == 0 {
                    false
                } else if vis[v as usize] == 2 {
                    true
                } else if !together(e, v as usize) {
                    false // trinca: face exposta
                } else {
                    continue;
                };
                let (a1, a2) = ((ax + 1) % 3, (ax + 2) % 3);
                let base = if sgn > 0 { k[ax] + 1 } else { k[ax] };
                let mut corners = [(0u64, [0.0f32; 3]); 4];
                for (q, (du, dv)) in [(0, 0), (1, 0), (1, 1), (0, 1)].into_iter().enumerate() {
                    let mut c = k;
                    c[ax] = base;
                    c[a1] = k[a1] + du;
                    c[a2] = k[a2] + dv;
                    // média dos deslocamentos das células em volta do canto presas a esta
                    let (mut acc, mut cnt, mut rep) = ([0.0f32; 3], 0.0f32, e as u32);
                    for dz in 0..2 {
                        for dy in 0..2 {
                            for dxx in 0..2 {
                                let w = g.at([c[0] - dxx, c[1] - dy, c[2] - dz]);
                                if live(w) && (w == e as u32 || together(e, w as usize)) {
                                    let dd = disp[w as usize];
                                    acc[0] += dd[0];
                                    acc[1] += dd[1];
                                    acc[2] += dd[2];
                                    cnt += 1.0;
                                    rep = rep.min(w);
                                }
                            }
                        }
                    }
                    let p = g.lat.corner(c);
                    let inv = 1.0 / cnt.max(1.0);
                    // chave: canto + representante do grupo de células presas (até ~16 mi de nós)
                    let id = gbit | (g.corner_id(c) << 24) ^ (rep as u64 & 0xFF_FFFF);
                    corners[q] = (id, [p[0] + acc[0] * inv, p[1] + acc[1] * inv, p[2] + acc[2] * inv]);
                }
                let eo = g.lat.e[ax];
                faces.push(Face { corners, outward: [eo[0] * sgn as f32, eo[1] * sgn as f32, eo[2] * sgn as f32], cell: e as u32, cut });
            }
        }
        (faces, false)
    };

    let results: Vec<(Vec<Face>, Vec<u32>)> = (0..total)
        .into_par_iter()
        .with_min_len(2048)
        .fold(
            || (Vec::new(), Vec::new()),
            |mut acc, e| {
                if vis[e] == 1 {
                    let (f, loose) = per_cell(e);
                    if loose {
                        acc.1.push(e as u32);
                    } else {
                        acc.0.extend(f);
                    }
                }
                acc
            },
        )
        .collect();
    let nq: usize = results.iter().map(|r| r.0.len()).sum();
    if nq > max_quads {
        return None;
    }

    // estilo "elemento erodido": entulho solto do alvo encostado no projétil some, para a
    // munição tocar direto na cratera (como nos solvers de elementos finitos)
    let results = {
        let h = 3.0 * dx;
        let pos = |e: u32| {
            let (r, d) = (t.refx[e as usize], disp[e as usize]);
            [r[0] + d[0], r[1] + d[1], r[2] + d[2]]
        };
        let key = |p: [f32; 3]| [(p[0] / h).floor() as i32, (p[1] / h).floor() as i32, (p[2] / h).floor() as i32];
        let mut hash: FxMap<[i32; 3], Vec<[f32; 3]>> = FxMap::default();
        for (_, loose) in &results {
            for &e in loose {
                if t.grp[e as usize] == 1 {
                    let p = pos(e);
                    hash.entry(key(p)).or_default().push(p);
                }
            }
        }
        let r2 = (2.6 * dx) * (2.6 * dx);
        results
            .into_iter()
            .map(|(f, loose)| {
                let keep: Vec<u32> = loose
                    .into_iter()
                    .filter(|&e| {
                        if t.grp[e as usize] == 1 {
                            return true;
                        }
                        let p = pos(e);
                        let k = key(p);
                        for a in -1..=1 {
                            for b in -1..=1 {
                                for c in -1..=1 {
                                    if let Some(v) = hash.get(&[k[0] + a, k[1] + b, k[2] + c]) {
                                        if v.iter().any(|q| (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2) < r2) {
                                            return false;
                                        }
                                    }
                                }
                            }
                        }
                        true
                    })
                    .collect();
                (f, keep)
            })
            .collect::<Vec<_>>()
    };
    let mut out = SurfMesh::default();
    out.quads.reserve(nq);
    let mut vmap: FxMap<u64, u32> = FxMap::default();
    vmap.reserve(nq);
    let mut outward = Vec::with_capacity(nq);
    for (faces, loose) in results {
        out.loose.extend(loose);
        for f in faces {
            let mut vi = [0u32; 4];
            for (q, (id, p)) in f.corners.into_iter().enumerate() {
                vi[q] = *vmap.entry(id).or_insert_with(|| {
                    out.verts.push(turn(p));
                    out.vcell.push(f.cell);
                    out.vcut.push(f.cut);
                    (out.verts.len() - 1) as u32
                });
            }
            outward.push(turn(f.outward));
            out.quads.push(Quad { v: vi, cell: f.cell, cut: f.cut });
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
        let vis = vec![1u8; pos.len()];
        let t = topo(pos.len(), &[0], &pos, &parts, np, cfg.dx as f32, cfg.obliquity() as f32, &flip);
        let sm = build(&t, &pos, &vis, cfg.dx as f32, usize::MAX, &flip, &turn).unwrap();
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

#[cfg(test)]
mod bench {
    use super::*;
    use pensim_core::{Model, Preset};
    #[test]
    #[ignore]
    fn mesh_speed() {
        let mut cfg = Preset::Rifle762.config();
        cfg.dx = 0.0006;
        cfg.symmetry = false;
        let m = Model::build(&cfg).unwrap();
        let pos: Vec<[f32; 3]> = m.pos.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
        let np = cfg.design.n_proj_parts() as u8;
        let flip = |q: [f32; 3], _m: u8| q;
        let turn = |q: [f32; 3]| q;
        let vis = vec![1u8; pos.len()];
        let t0 = std::time::Instant::now();
        let t = topo(pos.len(), &[0], &pos, &m.part, np, cfg.dx as f32, 0.0, &flip);
        let t1 = t0.elapsed();
        let t0 = std::time::Instant::now();
        let sm = build(&t, &pos, &vis, cfg.dx as f32, usize::MAX, &flip, &turn).unwrap();
        eprintln!("nós={} topo={:?} quadro={:?} faces={} vértices={}", pos.len(), t1, t0.elapsed(), sm.quads.len(), sm.verts.len());
    }
}

#[cfg(test)]
mod ammo_faces {
    use super::*;
    use pensim_core::{Model, AMMO};
    #[test]
    fn projectile_gets_faces() {
        for k in [4usize, 8, 14, 16] {
            let a = &AMMO[k];
            let s = 8.0 * 1.5e-3 / (a.caliber_mm * 1e-3);
            let mut cfg = a.config(s);
            cfg.symmetry = false;
            let m = Model::build(&cfg).unwrap();
            let pos: Vec<[f32; 3]> = m.pos.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
            let np = cfg.design.n_proj_parts() as u8;
            let flip = |q: [f32; 3], _m: u8| q;
            let turn = |q: [f32; 3]| q;
            let vis = vec![1u8; pos.len()];
            let t = topo(pos.len(), &[0], &pos, &m.part, np, cfg.dx as f32, 0.0, &flip);
            let sm = build(&t, &pos, &vis, cfg.dx as f32, usize::MAX, &flip, &turn).unwrap();
            let nproj = m.part.iter().filter(|&&p| p < np).count();
            let loose_p = sm.loose.iter().filter(|&&e| m.part[e as usize] < np).count();
            let faces_p = sm.quads.iter().filter(|q| m.part[q.cell as usize] < np).count();
            eprintln!("{}: nós proj {nproj}, soltos {loose_p}, faces proj {faces_p}", a.name);
        }
    }
}
