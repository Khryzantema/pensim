//! Projeto do app (`Config`) → malha de hexaedros do núcleo novo.
//!
//! Convenções iguais às do simulador antigo, para a interface não mudar: chapas empilhadas em x
//! a partir de x = 0, centradas em y, z; projétil voando na direção d = (cos α, 0, −sen α);
//! simetria em y = 0 sempre e em z = 0 só no impacto normal sem objetos.
use super::material::{from_library, FemMat};
use super::mesh::{self, HexMesh, Symmetry};
use crate::config::Config;

pub struct Setup {
    pub mesh: HexMesh,
    pub mats: Vec<FemMat>,
    pub v0: Vec<[f64; 3]>,
    /// peças [0, n_proj) são o projétil
    pub n_proj: u8,
    /// face traseira do alvo (x)
    pub back_x: f64,
    pub dir: [f64; 3],
    /// bits de simetria como no simulador antigo: 1 = y, 2 = z
    pub sym: u8,
}

pub fn build(cfg: &Config) -> Result<Setup, String> {
    let d_cfg = &cfg.design;
    if d_cfg.layers.is_empty() && d_cfg.objects.is_empty() {
        return Err("O alvo precisa de ao menos uma camada ou um objeto.".into());
    }
    let pr = &d_cfg.projectile;
    if pr.profile.len() < 2 {
        return Err("O esboço do projétil precisa de ao menos 2 pontos.".into());
    }
    let dx = cfg.dx;
    let alpha = cfg.obliquity();
    let spin = cfg.spin != 0.0;
    let symz = cfg.symmetry && !spin && alpha.abs() < 1e-9 && d_cfg.objects.is_empty();
    let sym = Symmetry { y0: cfg.symmetry && !spin, z0: symz };
    let n_proj = d_cfg.n_proj_parts();
    let lib_mats = d_cfg.materials();
    let mut mats: Vec<FemMat> = lib_mats[..n_proj].iter().map(from_library).collect();
    let mut all = HexMesh::default();

    // --- camadas
    let mut cursor = 0.0;
    for l in &d_cfg.layers {
        cursor += (l.gap_before / dx).round() * dx;
        let nt = ((l.thickness / dx).round() as usize).max(1);
        let t = nt as f64 * dx;
        let hw = ((l.width / 2.0 / dx).round().max(1.0)) * dx;
        let hh = if l.round { hw } else { ((l.height / 2.0 / dx).round().max(1.0)) * dx };
        let part = mats.len() as u8;
        let x0 = cursor;
        let mut m = if l.round {
            mesh::from_grid([x0, -hw, -hw], [x0 + t, hw, hw], dx, sym, part, &|c| c[0] > x0 && c[0] < x0 + t && c[1] * c[1] + c[2] * c[2] < hw * hw, &|p, _| p)
        } else {
            mesh::block([x0, -hw, -hh], [x0 + t, hw, hh], dx, sym, part)
        };
        if cfg.clamp_edges {
            for i in 0..m.n_nodes() {
                let p = m.x[i];
                let edge = if l.round { (p[1] * p[1] + p[2] * p[2]).sqrt() > hw - 1.01 * dx } else { p[1].abs() > hw - 1e-9 * hw || p[2].abs() > hh - 1e-9 * hh };
                if edge {
                    m.sym[i] |= 7;
                }
            }
        }
        all.append(&m);
        mats.push(from_library(&l.material));
        cursor += t;
    }
    let mut back_x = cursor;

    // --- objetos desenhados: contornos no plano x–z extrudados em y
    let mut xmin_all = 0.0f64;
    for o in &d_cfg.objects {
        let b = o.bbox();
        if b[0] > b[1] {
            continue;
        }
        let base = mats.len();
        let hw = ((o.width / 2.0 / dx).round().max(1.0)) * dx;
        let mut m = mesh::from_grid([b[0], -hw, b[2]], [b[1], hw, b[3]], dx, sym, 0, &|c| c[1].abs() < hw && o.part_at(c[0], c[2]).is_some(), &|p, _| p);
        for (e, h) in m.hex.iter().enumerate() {
            let c = (0..3).map(|a| h.iter().map(|&k| m.x[k as usize][a]).sum::<f64>() / 8.0).collect::<Vec<_>>();
            m.part[e] = (base + o.part_at(c[0], c[2]).unwrap_or(0)) as u8;
        }
        for p in &m.x {
            xmin_all = xmin_all.min(p[0]);
            back_x = back_x.max(p[0]);
        }
        all.append(&m);
        mats.extend(o.materials().iter().map(from_library));
    }
    if xmin_all < 0.0 {
        let sh = (-xmin_all / dx).ceil() * dx;
        all.translate([sh, 0.0, 0.0]);
        back_x += sh;
    }
    if all.n_elems() == 0 {
        return Err("Alvo vazio: aumente a resolução.".into());
    }
    let n_target_nodes = all.n_nodes();

    // --- projétil: revolução em O no eixo local a (0 = ponta), depois girado para a direção de voo
    let len = pr.length();
    let rmax = pr.max_radius();
    let nd = {
        let v = ((2.0 * rmax / dx).round() as usize).max(4);
        v + v % 2
    };
    let mut p = mesh::revolution_x(&|a| pr.radius_at(a), 0.0, len, nd, sym, 0);
    // peça de cada elemento pelo centróide; vazio sai
    let mut keep = vec![true; p.n_elems()];
    for (e, h) in p.hex.iter().enumerate() {
        let c = (0..3).map(|a| h.iter().map(|&k| p.x[k as usize][a]).sum::<f64>() / 8.0).collect::<Vec<_>>();
        let rho = (c[1] * c[1] + c[2] * c[2]).sqrt();
        match pr.part_at(c[0], rho, dx) {
            Some(part) => p.part[e] = part,
            None => keep[e] = false,
        }
    }
    if keep.iter().any(|k| !k) {
        let mut used = vec![u32::MAX; p.n_nodes()];
        let mut q = HexMesh::default();
        for (e, h) in p.hex.iter().enumerate() {
            if !keep[e] {
                continue;
            }
            q.hex.push(h.map(|k| {
                if used[k as usize] == u32::MAX {
                    used[k as usize] = q.x.len() as u32;
                    q.x.push(p.x[k as usize]);
                    q.sym.push(p.sym[k as usize]);
                }
                used[k as usize]
            }));
            q.part.push(p.part[e]);
        }
        p = q;
    }
    if p.n_elems() == 0 {
        return Err("Malha grossa demais para o projétil (nenhum elemento). Aumente a resolução.".into());
    }
    let d = [alpha.cos(), 0.0, -alpha.sin()];
    let ev = [alpha.sin(), 0.0, alpha.cos()];
    // velocidade de giro em torno do eixo de voo: ω d × r = ω (u·ev − v·ey), com (u, v) locais
    let spin_v: Vec<[f64; 3]> = p.x.iter().map(|q| {
        let (u, v) = (q[1], q[2]);
        [cfg.spin * u * ev[0], -cfg.spin * v, cfg.spin * u * ev[2]]
    }).collect();
    // local (a, u = y, v = z) → mundo: −d·a + ev·v, u
    for q in &mut p.x {
        let (a, u, v) = (q[0], q[1], q[2]);
        *q = [-d[0] * a + ev[0] * v, u, -d[2] * a + ev[2] * v];
    }
    // folga ao alvo: meio elemento + 5% do raio, deslocando AO LONGO da linha de voo
    let maxx = p.x.iter().map(|q| q[0]).fold(f64::MIN, f64::max);
    let gap = 0.5 * dx + 0.05 * rmax;
    let lambda = (maxx + gap) / d[0].max(0.05);
    for q in &mut p.x {
        q[0] -= d[0] * lambda;
        q[2] -= d[2] * lambda;
    }
    p.hex = p.hex.iter().map(|h| mesh::orient(&p.x, *h)).collect();
    all.append(&p);
    let n = all.n_nodes();
    let v0: Vec<[f64; 3]> = (0..n)
        .map(|i| {
            if i >= n_target_nodes {
                let sv = spin_v[i - n_target_nodes];
                [cfg.velocity * d[0] + sv[0], sv[1], cfg.velocity * d[2] + sv[2]]
            } else {
                [0.0; 3]
            }
        })
        .collect();
    // as peças do alvo começam depois das do projétil: renumera (o alvo foi montado primeiro)
    let nt_parts = mats.len() - n_proj;
    let _ = nt_parts;
    Ok(Setup { mesh: all, mats, v0, n_proj: n_proj as u8, back_x, dir: d, sym: (sym.y0 as u8) | ((sym.z0 as u8) << 1) })
}
