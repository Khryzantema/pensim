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

pub fn det3(j: &[[f64; 3]; 3]) -> f64 {
    j[0][0] * (j[1][1] * j[2][2] - j[1][2] * j[2][1]) - j[0][1] * (j[1][0] * j[2][2] - j[1][2] * j[2][0]) + j[0][2] * (j[1][0] * j[2][1] - j[1][1] * j[2][0])
}

// ------------------------------------------------------------------ formas prontas

/// Bloco [lo, hi] (as faces caem em planos de nós se lo/hi forem múltiplos de dx).
pub fn block(lo: [f64; 3], hi: [f64; 3], dx: f64, sym: Symmetry, part: u8) -> HexMesh {
    let lo2 = [lo[0], if sym.y0 { 0.0 } else { lo[1] }, if sym.z0 { 0.0 } else { lo[2] }];
    from_grid(lo2, hi, dx, sym, part, &|c| (0..3).all(|a| c[a] > lo2[a] && c[a] < hi[a]), &|p, _| p)
}

/// Cilindro de raio `r` no eixo x, de x0 a x1, encaixado na superfície lateral.
pub fn cylinder_x(r: f64, x0: f64, x1: f64, dx: f64, sym: Symmetry, part: u8) -> HexMesh {
    let lat = dir_bit(1, false) | dir_bit(1, true) | dir_bit(2, false) | dir_bit(2, true);
    from_grid(
        [x0, -r, -r],
        [x1, r, r],
        dx,
        sym,
        part,
        &|c| c[0] > x0 && c[0] < x1 && c[1] * c[1] + c[2] * c[2] < r * r,
        &|p, dirs| {
            let mut q = p;
            if dirs & lat != 0 {
                let rr = (p[1] * p[1] + p[2] * p[2]).sqrt();
                if rr > 1e-12 {
                    q[1] = p[1] * r / rr;
                    q[2] = p[2] * r / rr;
                }
            }
            if dirs & dir_bit(0, false) != 0 {
                q[0] = x0;
            }
            if dirs & dir_bit(0, true) != 0 {
                q[0] = x1;
            }
            q
        },
    )
}

/// Esfera de raio `r` centrada em `c` (c[1] = c[2] = 0 quando há simetria).
pub fn sphere(r: f64, c: [f64; 3], dx: f64, sym: Symmetry, part: u8) -> HexMesh {
    from_grid(
        [c[0] - r, c[1] - r, c[2] - r],
        [c[0] + r, c[1] + r, c[2] + r],
        dx,
        sym,
        part,
        &|p| (0..3).map(|a| (p[a] - c[a]).powi(2)).sum::<f64>() < r * r,
        &|p, _| {
            let d = [p[0] - c[0], p[1] - c[1], p[2] - c[2]];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-12);
            [c[0] + d[0] * r / l, c[1] + d[1] * r / l, c[2] + d[2] * r / l]
        },
    )
}
