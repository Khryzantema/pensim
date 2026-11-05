//! Voxelização do projeto (CAD) em nós de uma rede cúbica.
use crate::config::Config;
use crate::material::Material;

pub struct Model {
    pub pos: Vec<[f64; 3]>,
    /// 0 = projétil, 1.. = camadas
    pub part: Vec<u8>,
    pub fixed: Vec<bool>,
    pub mats: Vec<Material>,
    pub back_x: f64,
    pub dir: [f64; 3],
    /// peças [0, n_proj) são o projétil
    pub n_proj: u8,
    /// planos de simetria ativos: bit 1 = y, bit 2 = z (só os nós y>0 / z>0 são gerados)
    pub sym: u8,
}

impl Model {
    /// Modelo reduzido por simetria (se `cfg.symmetry`): só os nós y > 0 (e z > 0 no impacto normal).
    pub fn build(cfg: &Config) -> Result<Model, String> {
        let mut m = Self::build_full(cfg)?;
        if cfg.symmetry {
            let sym: u8 = 1 | if cfg.obliquity_deg.abs() < 1e-9 { 2 } else { 0 };
            let keep: Vec<bool> = m.pos.iter().map(|q| q[1] > 0.0 && (sym & 2 == 0 || q[2] > 0.0)).collect();
            let sel = |v: &Vec<[f64; 3]>| -> Vec<[f64; 3]> { v.iter().zip(&keep).filter(|(_, k)| **k).map(|(q, _)| *q).collect() };
            m.pos = sel(&m.pos);
            m.part = m.part.iter().zip(&keep).filter(|(_, k)| **k).map(|(p, _)| *p).collect();
            m.fixed = m.fixed.iter().zip(&keep).filter(|(_, k)| **k).map(|(f, _)| *f).collect();
            m.sym = sym;
        }
        Ok(m)
    }

    /// Hash determinístico → [-1, 1] (splitmix64), para jitter reprodutível.
pub fn hash_unit(a: u64, b: u64, c: u64, d: u64) -> f64 {
    let mut z = a.wrapping_mul(0x9E3779B97F4A7C15) ^ b.wrapping_mul(0xBF58476D1CE4E5B9) ^ c.wrapping_mul(0x94D049BB133111EB) ^ d.wrapping_mul(0xD6E8FEB86659FD93);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
    z ^= z >> 31;
    (z >> 11) as f64 / (1u64 << 52) as f64 - 1.0
}

    /// Modelo completo (pré-visualização e comparação).
    pub fn build_full(cfg: &Config) -> Result<Model, String> {
        let dx = cfg.dx;
        if cfg.design.layers.is_empty() {
            return Err("O alvo precisa de ao menos uma camada.".into());
        }
        if cfg.design.projectile.profile.len() < 2 {
            return Err("O esboço do projétil precisa de ao menos 2 pontos.".into());
        }
        let mut pos = vec![];
        let mut part = vec![];
        let mut fixed = vec![];
        let n_proj = cfg.design.n_proj_parts();
        let mut mats = cfg.design.materials();
        mats.truncate(n_proj);

        // --- camadas do alvo (eixo x = normal; centradas em y,z)
        let mut cursor = 0.0;
        for (li, l) in cfg.design.layers.iter().enumerate() {
            cursor += (l.gap_before / dx).round() * dx;
            let nt = ((l.thickness / dx).round() as usize).max(1);
            let even = |v: usize| v + v % 2;
            let ny = even(((l.width / dx).round() as usize).max(2));
            let nz = if l.round { ny } else { even(((l.height / dx).round() as usize).max(2)) };
            let r = ny as f64 * dx / 2.0;
            for i in 0..nt {
                let x = cursor + (i as f64 + 0.5) * dx;
                for j in 0..ny {
                    let y = (j as f64 + 0.5 - ny as f64 / 2.0) * dx;
                    for k in 0..nz {
                        let z = (k as f64 + 0.5 - nz as f64 / 2.0) * dx;
                        if l.round && y * y + z * z > r * r {
                            continue;
                        }
                        let edge = if l.round {
                            (y * y + z * z).sqrt() > r - 2.0 * dx
                        } else {
                            y.abs() > ny as f64 * dx / 2.0 - 2.0 * dx || z.abs() > nz as f64 * dx / 2.0 - 2.0 * dx
                        };
                        let jt = cfg.jitter * dx;
                        let id = ((li as u64) << 40) | ((i as u64) << 20) | j as u64;
                        pos.push([x + jt * Self::hash_unit(1, id, k as u64, 1), y + jt * Self::hash_unit(1, id, k as u64, 2), z + jt * Self::hash_unit(1, id, k as u64, 3)]);
                        part.push((n_proj + li) as u8);
                        fixed.push(cfg.clamp_edges && edge);
                    }
                }
            }
            mats.push(l.material.clone());
            cursor += nt as f64 * dx;
        }
        let back_x = cursor;

        // --- projétil: sólido de revolução em torno do eixo de voo
        let pr = &cfg.design.projectile;
        let len = pr.length();
        let rmax = pr.max_radius();
        let na = ((len / dx).round() as usize).max(2);
        let nw = {
            let v = (((2.0 * rmax) / dx).round() as usize).max(2);
            v + v % 2
        };
        let alpha = cfg.obliquity();
        let d = [alpha.cos(), 0.0, -alpha.sin()];
        let ev = [alpha.sin(), 0.0, alpha.cos()];
        let mut ploc: Vec<(f64, f64, f64, u8)> = vec![];
        for ia in 0..na {
            let a = (ia as f64 + 0.5) * len / na as f64;
            let ra = pr.radius_at(a);
            for iu in 0..nw {
                let u = (iu as f64 - (nw as f64 - 1.0) / 2.0) * dx;
                for iv in 0..nw {
                    let v = (iv as f64 - (nw as f64 - 1.0) / 2.0) * dx;
                    let rho = (u * u + v * v).sqrt();
                    if rho <= ra + 1e-12 {
                        let in_core = pr.core.as_ref().map_or(false, |c| rho <= crate::config::profile_radius(&c.profile, a) + 1e-12);
                        ploc.push((a, u, v, in_core as u8));
                    }
                }
            }
        }
        if ploc.is_empty() {
            return Err("Malha grossa demais para o projétil (nenhum nó). Reduza o espaçamento.".into());
        }
        let wp: Vec<[f64; 3]> = ploc
            .iter()
            .map(|&(a, u, v, _)| [-d[0] * a + ev[0] * v, u, -d[2] * a + ev[2] * v])
            .collect();
        let maxx = wp.iter().map(|p| p[0]).fold(f64::MIN, f64::max);
        let shift = -0.6 * dx - maxx;
        for (k, p) in wp.into_iter().enumerate() {
            let jt = cfg.jitter * dx;
            let kk = k as u64;
            pos.push([p[0] + shift + jt * Self::hash_unit(99, kk, 0, 1), p[1] + jt * Self::hash_unit(99, kk, 0, 2), p[2] + jt * Self::hash_unit(99, kk, 0, 3)]);
            part.push(ploc[k].3);
            fixed.push(false);
        }
        Ok(Model { pos, part, fixed, mats, back_x, dir: d, n_proj: n_proj as u8, sym: 0 })
    }
}
