//! Malha de hexaedros de 8 nós a partir de uma grade estruturada, com encaixe da borda na
//! superfície real (sem degraus) e planos de simetria.
//!
//! Ordem dos nós no elemento (sinais de ξ, η, ζ):
//! 0 (−,−,−) 1 (+,−,−) 2 (+,+,−) 3 (−,+,−) 4 (−,−,+) 5 (+,−,+) 6 (+,+,+) 7 (−,+,+)
use std::collections::HashMap;

/// Sinais (ξ, η, ζ) de cada nó do hexaedro.
pub const NODE_SIGNS: [[f64; 3]; 8] = [
    [-1.0, -1.0, -1.0],
    [1.0, -1.0, -1.0],
    [1.0, 1.0, -1.0],
    [-1.0, 1.0, -1.0],
    [-1.0, -1.0, 1.0],
    [1.0, -1.0, 1.0],
    [1.0, 1.0, 1.0],
    [-1.0, 1.0, 1.0],
];

/// As 6 faces do hexaedro (nós locais em ordem anti-horária vista de fora).
pub const FACES: [[usize; 4]; 6] = [
    [0, 3, 2, 1], // −ζ
    [4, 5, 6, 7], // +ζ
    [0, 1, 5, 4], // −η
    [2, 3, 7, 6], // +η
    [0, 4, 7, 3], // −ξ
    [1, 2, 6, 5], // +ξ
];

/// Direção da normal externa de cada face na grade: (eixo, sinal).
const FACE_DIR: [(usize, i32); 6] = [(2, -1), (2, 1), (1, -1), (1, 1), (0, -1), (0, 1)];

/// Bits de direção de face exposta: 2·eixo + (sinal > 0)
pub fn dir_bit(axis: usize, positive: bool) -> u8 {
    1 << (2 * axis + positive as usize)
}

#[derive(Clone, Debug, Default)]
pub struct HexMesh {
    pub x: Vec<[f64; 3]>,
    pub hex: Vec<[u32; 8]>,
    /// peça (material) de cada elemento
    pub part: Vec<u8>,
    /// bits de eixo fixo por simetria (1 = x, 2 = y, 4 = z) por nó
    pub sym: Vec<u8>,
}

/// Planos de simetria (só pelos planos coordenados que passam pela origem).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Symmetry {
    pub y0: bool,
    pub z0: bool,
}

impl HexMesh {
    pub fn n_nodes(&self) -> usize {
        self.x.len()
    }
    pub fn n_elems(&self) -> usize {
        self.hex.len()
    }

    /// Volume total (soma dos volumes exatos dos hexaedros).
    pub fn volume(&self) -> f64 {
        self.hex.iter().map(|h| hex_volume(&self.corners(h))).sum()
    }

    pub fn corners(&self, h: &[u32; 8]) -> [[f64; 3]; 8] {
        std::array::from_fn(|k| self.x[h[k] as usize])
    }

    /// Menor razão jacobiano/nominal entre os 8 cantos de todos os elementos.
    pub fn min_jacobian_ratio(&self, dx: f64) -> f64 {
        let nominal = (dx / 2.0).powi(3);
        self.hex
            .iter()
            .map(|h| corner_jacobian_min(&self.corners(h)) / nominal)
            .fold(f64::INFINITY, f64::min)
    }

    /// Menor jacobiano escalonado da malha (forma dos elementos).
    pub fn min_scaled_jacobian(&self) -> f64 {
        self.hex.iter().map(|h| scaled_jacobian_min(&self.corners(h))).fold(f64::INFINITY, f64::min)
    }

    /// Junta outra malha (nós não compartilhados).
    pub fn append(&mut self, o: &HexMesh) {
        let off = self.x.len() as u32;
        self.x.extend_from_slice(&o.x);
        self.sym.extend_from_slice(&o.sym);
        self.part.extend_from_slice(&o.part);
        self.hex.extend(o.hex.iter().map(|h| h.map(|k| k + off)));
    }

    /// Desloca todos os nós.
    pub fn translate(&mut self, d: [f64; 3]) {
        for p in &mut self.x {
            for a in 0..3 {
                p[a] += d[a];
            }
        }
    }
}

/// Gera a malha das células da grade de passo `dx` cujo centro satisfaz `inside`. A grade tem
/// nós em múltiplos inteiros de `dx` (logo os planos y = 0 e z = 0 são planos de nós).
///
/// `snap(p, dirs)` recebe um nó da superfície e os bits das direções das faces expostas que o
/// tocam; devolve a posição na superfície real. Faces sobre planos de simetria não contam como
/// superfície.
pub fn from_grid(
    lo: [f64; 3],
    hi: [f64; 3],
    dx: f64,
    sym: Symmetry,
    part: u8,
    inside: &dyn Fn([f64; 3]) -> bool,
    snap: &dyn Fn([f64; 3], u8) -> [f64; 3],
) -> HexMesh {
    let i0: [i64; 3] = std::array::from_fn(|a| (lo[a] / dx).floor() as i64 - 1);
    let i1: [i64; 3] = std::array::from_fn(|a| (hi[a] / dx).ceil() as i64 + 1);
    // células dentro
    let mut cells: Vec<[i64; 3]> = Vec::new();
    for i in i0[0]..i1[0] {
        for j in i0[1]..i1[1] {
            if sym.y0 && j < 0 {
                continue;
            }
            for k in i0[2]..i1[2] {
                if sym.z0 && k < 0 {
                    continue;
                }
                let c = [(i as f64 + 0.5) * dx, (j as f64 + 0.5) * dx, (k as f64 + 0.5) * dx];
                if inside(c) {
                    cells.push([i, j, k]);
                }
            }
        }
    }
    let cell_set: std::collections::HashSet<[i64; 3]> = cells.iter().copied().collect();
    let mut node_id: HashMap<[i64; 3], u32> = HashMap::new();
    let mut m = HexMesh::default();
    let mut dirs: Vec<u8> = Vec::new();
    for c in &cells {
        let mut h = [0u32; 8];
        for (ln, s) in NODE_SIGNS.iter().enumerate() {
            let g = [c[0] + (s[0] > 0.0) as i64, c[1] + (s[1] > 0.0) as i64, c[2] + (s[2] > 0.0) as i64];
            let id = *node_id.entry(g).or_insert_with(|| {
                m.x.push([g[0] as f64 * dx, g[1] as f64 * dx, g[2] as f64 * dx]);
                let mut b = 0u8;
                if sym.y0 && g[1] == 0 {
                    b |= 2;
                }
                if sym.z0 && g[2] == 0 {
                    b |= 4;
                }
                m.sym.push(b);
                dirs.push(0);
                (m.x.len() - 1) as u32
            });
            h[ln] = id;
        }
        // faces expostas (vizinho ausente) que não estão num plano de simetria
        for (f, &(ax, sg)) in FACES.iter().zip(FACE_DIR.iter()) {
            let mut nb = *c;
            nb[ax] += sg as i64;
            if cell_set.contains(&nb) {
                continue;
            }
            let on_sym = (ax == 1 && sg < 0 && sym.y0 && c[1] == 0) || (ax == 2 && sg < 0 && sym.z0 && c[2] == 0);
            if on_sym {
                continue;
            }
            let bit = dir_bit(ax, sg > 0);
            for &ln in f {
                dirs[h[ln] as usize] |= bit;
            }
        }
        m.hex.push(h);
        m.part.push(part);
    }
    // encaixe da borda
    let n = m.x.len();
    let mut on_surf = vec![false; n];
    for i in 0..n {
        if dirs[i] != 0 {
            let p = snap(m.x[i], dirs[i]);
            // nó de simetria não sai do plano
            let mut q = p;
            if m.sym[i] & 2 != 0 {
                q[1] = 0.0;
            }
            if m.sym[i] & 4 != 0 {
                q[2] = 0.0;
            }
            m.x[i] = q;
            on_surf[i] = true;
        }
    }
    smooth_near_surface(&mut m, &on_surf, 3);
    m
}

/// Suavização laplaciana dos nós internos vizinhos da borda (corrige elementos achatados pelo
/// encaixe). Nós da superfície e de simetria (na componente fixa) ficam onde estão.
fn smooth_near_surface(m: &mut HexMesh, on_surf: &[bool], passes: usize) {
    let n = m.x.len();
    // vizinhos por aresta
    let mut nb: Vec<Vec<u32>> = vec![Vec::new(); n];
    const EDGES: [(usize, usize); 12] = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)];
    for h in &m.hex {
        for &(a, b) in &EDGES {
            let (a, b) = (h[a], h[b]);
            if !nb[a as usize].contains(&b) {
                nb[a as usize].push(b);
            }
            if !nb[b as usize].contains(&a) {
                nb[b as usize].push(a);
            }
        }
    }
    let near: Vec<bool> = (0..n).map(|i| !on_surf[i] && nb[i].iter().any(|&j| on_surf[j as usize])).collect();
    for _ in 0..passes {
        let old = m.x.clone();
        for i in 0..n {
            if !near[i] {
                continue;
            }
            let mut acc = [0.0; 3];
            for &j in &nb[i] {
                for a in 0..3 {
                    acc[a] += old[j as usize][a];
                }
            }
            let k = nb[i].len() as f64;
            for a in 0..3 {
                if a == 1 && m.sym[i] & 2 != 0 || a == 2 && m.sym[i] & 4 != 0 {
                    continue;
                }
                m.x[i][a] = 0.5 * old[i][a] + 0.5 * acc[a] / k;
            }
        }
    }
}

/// Volume exato de um hexaedro trilinear (fórmula de Grandy / decomposição em 5 tetraedros não é
/// exata para faces empenadas; esta é: integral de det J com quadratura de 2×2×2 pontos).
pub fn hex_volume(x: &[[f64; 3]; 8]) -> f64 {
    let g = 1.0 / 3f64.sqrt();
    let mut v = 0.0;
    for &a in &[-g, g] {
        for &b in &[-g, g] {
            for &c in &[-g, g] {
                v += jacobian_det(x, [a, b, c]);
            }
        }
    }
    v
}

/// det J no ponto (ξ, η, ζ).
pub fn jacobian_det(x: &[[f64; 3]; 8], p: [f64; 3]) -> f64 {
    let mut j = [[0.0; 3]; 3];
    for (k, s) in NODE_SIGNS.iter().enumerate() {
        let d = [
            s[0] * (1.0 + s[1] * p[1]) * (1.0 + s[2] * p[2]) / 8.0,
            s[1] * (1.0 + s[0] * p[0]) * (1.0 + s[2] * p[2]) / 8.0,
            s[2] * (1.0 + s[0] * p[0]) * (1.0 + s[1] * p[1]) / 8.0,
        ];
        for a in 0..3 {
            for b in 0..3 {
                j[a][b] += x[k][a] * d[b];
            }
        }
    }
    det3(&j)
}

/// Menor det J entre os 8 cantos (qualidade: ≤ 0 = elemento invertido).
pub fn corner_jacobian_min(x: &[[f64; 3]; 8]) -> f64 {
    NODE_SIGNS.iter().map(|&s| jacobian_det(x, s)).fold(f64::INFINITY, f64::min)
}

/// Jacobiano escalonado mínimo (det das 3 arestas unitárias em cada canto): 1 = cubo perfeito,
/// ≤ 0 = invertido. Medida de forma, independente do tamanho.
pub fn scaled_jacobian_min(x: &[[f64; 3]; 8]) -> f64 {
    let mut worst = f64::INFINITY;
    for (k, s) in NODE_SIGNS.iter().enumerate() {
        // vizinhos ao longo de ξ, η, ζ
        let nb: [usize; 3] = std::array::from_fn(|a| {
            let mut t = *s;
            t[a] = -t[a];
            NODE_SIGNS.iter().position(|u| *u == t).unwrap()
        });
        let mut j = [[0.0; 3]; 3];
        for a in 0..3 {
            let d = [x[nb[a]][0] - x[k][0], x[nb[a]][1] - x[k][1], x[nb[a]][2] - x[k][2]];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-300);
            // orienta a aresta para o lado +ξ_a
            let sg = -s[a];
            for c in 0..3 {
                j[c][a] = sg * d[c] / l;
            }
        }
        worst = worst.min(det3(&j));
    }
    worst
}

pub fn det3(j: &[[f64; 3]; 3]) -> f64 {
    j[0][0] * (j[1][1] * j[2][2] - j[1][2] * j[2][1]) - j[0][1] * (j[1][0] * j[2][2] - j[1][2] * j[2][0]) + j[0][2] * (j[1][0] * j[2][1] - j[1][1] * j[2][0])
}

// ------------------------------------------------------------------ formas prontas

/// Bloco [lo, hi] (as faces caem em planos de nós se lo/hi forem múltiplos de dx).
pub fn block(lo: [f64; 3], hi: [f64; 3], dx: f64, sym: Symmetry, part: u8) -> HexMesh {
    let lo2 = [lo[0], if sym.y0 { 0.0 } else { lo[1] }, if sym.z0 { 0.0 } else { lo[2] }];
    from_grid(lo2, hi, dx, sym, part, &|c| (0..3).all(|a| c[a] > lo2[a] && c[a] < hi[a]), &|p, _| p)
}

/// Malha estruturada: nó (i, j, k) na posição `pos(i, j, k)`, células nos intervalos dados
/// (limites inclusivos dos NÓS). Nós com j = 0 / k = 0 levam o bit de simetria, se pedido.
pub fn structured(ni: (i64, i64), nj: (i64, i64), nk: (i64, i64), sym: Symmetry, part: u8, pos: &dyn Fn(i64, i64, i64) -> [f64; 3]) -> HexMesh {
    let (sx, sy, sz) = ((ni.1 - ni.0 + 1) as usize, (nj.1 - nj.0 + 1) as usize, (nk.1 - nk.0 + 1) as usize);
    let id = |i: i64, j: i64, k: i64| (((i - ni.0) as usize * sy + (j - nj.0) as usize) * sz + (k - nk.0) as usize) as u32;
    let mut m = HexMesh::default();
    m.x.reserve(sx * sy * sz);
    for i in ni.0..=ni.1 {
        for j in nj.0..=nj.1 {
            for k in nk.0..=nk.1 {
                m.x.push(pos(i, j, k));
                m.sym.push(if sym.y0 && j == 0 { 2 } else { 0 } | if sym.z0 && k == 0 { 4 } else { 0 });
            }
        }
    }
    for i in ni.0..ni.1 {
        for j in nj.0..nj.1 {
            for k in nk.0..nk.1 {
                let h: [u32; 8] = std::array::from_fn(|l| {
                    let s = NODE_SIGNS[l];
                    id(i + (s[0] > 0.0) as i64, j + (s[1] > 0.0) as i64, k + (s[2] > 0.0) as i64)
                });
                m.hex.push(h);
                m.part.push(part);
            }
        }
    }
    m
}

// ------------------------------------------------------------------ malha em O

/// Meia-aresta do núcleo cúbico (fração do raio) e quanto a borda dele é arredondada.
const BETA: f64 = 0.5;
const KAPPA: f64 = 0.4;

/// Ponto do núcleo: q no cubo [−1, 1]^d → núcleo arredondado de meia-aresta BETA.
fn core_point(q: [f64; 3]) -> [f64; 3] {
    let m = q.iter().fold(0.0f64, |a, &v| a.max(v.abs()));
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
    if l < 1e-15 {
        return [0.0; 3];
    }
    std::array::from_fn(|a| BETA * ((1.0 - KAPPA) * q[a] + KAPPA * q[a] / l * m))
}

/// Ponto da superfície (esfera/círculo unitário) na direção equiângula de q (q numa face do cubo).
fn surface_point(q: [f64; 3]) -> [f64; 3] {
    let e = q.map(|v| (std::f64::consts::FRAC_PI_4 * v).tan());
    let l = (e[0] * e[0] + e[1] * e[1] + e[2] * e[2]).sqrt().max(1e-300);
    e.map(|v| v / l)
}

/// Divisões: `c` células de meia-aresta do núcleo, `nr` camadas na casca (nd = 2c + 2nr).
fn o_split(nd: usize) -> (i64, i64) {
    let half = (nd / 2).max(2) as i64;
    let c = (half / 2).max(1);
    (c, half - c)
}

struct NodeMap {
    ids: HashMap<[i64; 3], u32>,
    pts: Vec<[f64; 3]>,
}

impl NodeMap {
    fn new() -> Self {
        NodeMap { ids: HashMap::new(), pts: Vec::new() }
    }
    fn id(&mut self, p: [f64; 3]) -> u32 {
        let k = p.map(|v| (v * 1e9).round() as i64);
        let n = self.pts.len() as u32;
        let id = *self.ids.entry(k).or_insert(n);
        if id == n {
            self.pts.push(p);
        }
        id
    }
}

/// Disco unitário em O no plano (a, b): quadriláteros anti-horários.
fn o_disk(nd: usize) -> (Vec<[f64; 2]>, Vec<[u32; 4]>) {
    let (c, nr) = o_split(nd);
    let mut nm = NodeMap::new();
    let mut quads = Vec::new();
    let mut push = |nm: &mut NodeMap, p: [[f64; 3]; 4]| {
        let ids = p.map(|q| nm.id(q));
        // área com sinal no plano
        let area: f64 = (0..4).map(|k| { let (a, b) = (p[k], p[(k + 1) % 4]); a[0] * b[1] - b[0] * a[1] }).sum();
        quads.push(if area > 0.0 { ids } else { [ids[3], ids[2], ids[1], ids[0]] });
    };
    let cf = c as f64;
    for a in -c..c {
        for b in -c..c {
            let p = [(a, b), (a + 1, b), (a + 1, b + 1), (a, b + 1)].map(|(u, v)| core_point([u as f64 / cf, v as f64 / cf, 0.0]));
            push(&mut nm, p);
        }
    }
    for ax in 0..2 {
        for sg in [-1.0, 1.0] {
            let pt = |u: i64, w: i64| {
                let mut q = [0.0; 3];
                q[ax] = sg;
                q[1 - ax] = u as f64 / cf;
                let (pi, po) = (core_point(q), surface_point(q));
                let t = w as f64 / nr as f64;
                std::array::from_fn(|k| pi[k] + t * (po[k] - pi[k]))
            };
            for u in -c..c {
                for w in 0..nr {
                    push(&mut nm, [pt(u, w), pt(u + 1, w), pt(u + 1, w + 1), pt(u, w + 1)]);
                }
            }
        }
    }
    (nm.pts.iter().map(|p| [p[0], p[1]]).collect(), quads)
}

/// Mantém só os elementos do lado positivo dos planos de simetria e marca os nós neles.
fn apply_symmetry(mut m: HexMesh, sym: Symmetry) -> HexMesh {
    if sym.y0 || sym.z0 {
        let keep: Vec<bool> = m
            .hex
            .iter()
            .map(|h| {
                let c = (0..3).map(|a| h.iter().map(|&k| m.x[k as usize][a]).sum::<f64>() / 8.0).collect::<Vec<_>>();
                !(sym.y0 && c[1] < 0.0) && !(sym.z0 && c[2] < 0.0)
            })
            .collect();
        let mut used = vec![u32::MAX; m.x.len()];
        let mut x = Vec::new();
        let mut hex = Vec::new();
        let mut part = Vec::new();
        for (e, h) in m.hex.iter().enumerate() {
            if !keep[e] {
                continue;
            }
            hex.push(h.map(|k| {
                if used[k as usize] == u32::MAX {
                    used[k as usize] = x.len() as u32;
                    x.push(m.x[k as usize]);
                }
                used[k as usize]
            }));
            part.push(m.part[e]);
        }
        m = HexMesh { x, hex, part, sym: vec![] };
    }
    let tol = 1e-9 * m.x.iter().map(|p| p[0].abs().max(p[1].abs()).max(p[2].abs())).fold(1e-12, f64::max);
    m.sym = m.x.iter().map(|p| (if sym.y0 && p[1].abs() < tol { 2 } else { 0 }) | (if sym.z0 && p[2].abs() < tol { 4 } else { 0 })).collect();
    for (p, &b) in m.x.iter_mut().zip(&m.sym) {
        if b & 2 != 0 {
            p[1] = 0.0;
        }
        if b & 4 != 0 {
            p[2] = 0.0;
        }
    }
    m
}

/// Põe o hexaedro com volume positivo (espelha a ordem se precisar).
fn orient(x: &[[f64; 3]], h: [u32; 8]) -> [u32; 8] {
    let c: [[f64; 3]; 8] = std::array::from_fn(|k| x[h[k] as usize]);
    if hex_volume(&c) >= 0.0 {
        h
    } else {
        [h[3], h[2], h[1], h[0], h[7], h[6], h[5], h[4]]
    }
}

/// Cilindro de raio `r` no eixo x, de x0 a x1, com `nd` elementos no diâmetro.
pub fn cylinder_x(r: f64, x0: f64, x1: f64, nd: usize, sym: Symmetry, part: u8) -> HexMesh {
    revolution_x(&|_| r, x0, x1, nd, sym, part)
}

/// Sólido de revolução no eixo x com raio `radius(x)`, `nd` elementos no maior diâmetro: seção
/// em O (núcleo + 4 blocos) extrudada, camadas com o passo do diâmetro. O raio tem um mínimo
/// de ¼ do passo para a ponta não colapsar.
pub fn revolution_x(radius: &dyn Fn(f64) -> f64, x0: f64, x1: f64, nd: usize, sym: Symmetry, part: u8) -> HexMesh {
    let rmax = (0..=200).map(|k| radius(x0 + (x1 - x0) * k as f64 / 200.0)).fold(0.0, f64::max);
    let dx = 2.0 * rmax / nd as f64;
    let nx = ((x1 - x0) / dx).round().max(1.0) as usize;
    let rmin = 0.25 * dx;
    let (disk, quads) = o_disk(nd);
    let nd2 = disk.len();
    let mut m = HexMesh::default();
    for i in 0..=nx {
        let x = x0 + (x1 - x0) * i as f64 / nx as f64;
        let r = radius(x).max(rmin);
        for p in &disk {
            m.x.push([x, p[0] * r, p[1] * r]);
        }
    }
    for i in 0..nx {
        for q in &quads {
            let lo = (i * nd2) as u32;
            let hi = ((i + 1) * nd2) as u32;
            // quadrilátero no plano (y, z) + camada em x
            let h = [q[0] + lo, q[1] + lo, q[2] + lo, q[3] + lo, q[0] + hi, q[1] + hi, q[2] + hi, q[3] + hi];
            m.hex.push(orient(&m.x, h));
            m.part.push(part);
        }
    }
    m.sym = vec![0; m.x.len()];
    apply_symmetry(m, sym)
}

/// Esfera de raio `r` centrada em `c` (c[1] = c[2] = 0 se houver simetria), `nd` elementos no
/// diâmetro: núcleo cúbico + 6 blocos de casca.
pub fn sphere(r: f64, c: [f64; 3], nd: usize, sym: Symmetry, part: u8) -> HexMesh {
    let (nc, nr) = o_split(nd);
    let cf = nc as f64;
    let mut nm = NodeMap::new();
    let mut cells: Vec<[u32; 8]> = Vec::new();
    let mut add = |nm: &mut NodeMap, p: [[f64; 3]; 8]| {
        let ids = p.map(|q| nm.id(q));
        cells.push(ids);
    };
    // núcleo
    for a in -nc..nc {
        for b in -nc..nc {
            for d in -nc..nc {
                let p: [[f64; 3]; 8] = std::array::from_fn(|l| {
                    let s = NODE_SIGNS[l];
                    let g = [a + (s[0] > 0.0) as i64, b + (s[1] > 0.0) as i64, d + (s[2] > 0.0) as i64];
                    core_point(g.map(|v| v as f64 / cf))
                });
                add(&mut nm, p);
            }
        }
    }
    // casca: 6 blocos
    for ax in 0..3 {
        let (o1, o2) = ((ax + 1) % 3, (ax + 2) % 3);
        for sg in [-1.0, 1.0] {
            let pt = |u: i64, v: i64, w: i64| -> [f64; 3] {
                let mut q = [0.0; 3];
                q[ax] = sg;
                q[o1] = u as f64 / cf;
                q[o2] = v as f64 / cf;
                let (pi, po) = (core_point(q), surface_point(q));
                let t = w as f64 / nr as f64;
                std::array::from_fn(|k| pi[k] + t * (po[k] - pi[k]))
            };
            for u in -nc..nc {
                for v in -nc..nc {
                    for w in 0..nr {
                        let p: [[f64; 3]; 8] = std::array::from_fn(|l| {
                            let s = NODE_SIGNS[l];
                            pt(u + (s[0] > 0.0) as i64, v + (s[1] > 0.0) as i64, w + (s[2] > 0.0) as i64)
                        });
                        add(&mut nm, p);
                    }
                }
            }
        }
    }
    let x: Vec<[f64; 3]> = nm.pts.iter().map(|p| [c[0] + r * p[0], c[1] + r * p[1], c[2] + r * p[2]]).collect();
    let hex: Vec<[u32; 8]> = cells.into_iter().map(|h| orient(&x, h)).collect();
    let n = hex.len();
    apply_symmetry(HexMesh { x, hex, part: vec![part; n], sym: vec![] }, sym)
}
