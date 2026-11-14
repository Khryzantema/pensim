//! Diagnóstico: fragmentos (nós sem elemento vivo) que ficam DENTRO de um elemento vivo de outra
//! peça. AMMO, A, NODES, V como em calib_ammo; STEPS = número de instantes verificados.
use pensim_core::fem::build::build;
use pensim_core::fem::mesh::FACES;
use pensim_core::fem::{Fem, FemModel};
use pensim_core::*;
use std::collections::HashMap;

fn env(k: &str, d: f64) -> f64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
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

/// profundidade (>0) do ponto dentro do hexaedro (aprox. por planos das faces), ou None
fn inside(p: [f64; 3], c: &[[f64; 3]; 8]) -> Option<f64> {
    let ec = [0, 1, 2].map(|a| c.iter().map(|q| q[a]).sum::<f64>() / 8.0);
    let mut dmin = f64::MAX;
    for f in FACES {
        let q = f.map(|l| c[l]);
        let cen = [0, 1, 2].map(|a| 0.25 * (q[0][a] + q[1][a] + q[2][a] + q[3][a]));
        let mut n = cross(sub(q[2], q[0]), sub(q[3], q[1]));
        let l = dot(n, n).sqrt();
        if l == 0.0 {
            return None;
        }
        n = n.map(|v| v / l);
        if dot(sub(cen, ec), n) < 0.0 {
            n = n.map(|v| -v);
        }
        let d = -dot(sub(p, cen), n);
        if d <= 0.0 {
            return None;
        }
        dmin = dmin.min(d);
    }
    Some(dmin)
}

fn main() {
    let key = std::env::var("AMMO").unwrap_or("M61".into());
    let a = AMMO.iter().find(|a| a.name.contains(&key)).expect("munição");
    let nodes = env("NODES", 12.0);
    let s = nodes * 1.5e-3 / (a.caliber_mm * 1e-3);
    let mut c = a.config(s);
    c.dx = 1.5e-3;
    c.obliquity_deg = env("A", 0.0);
    c.velocity = env("V", a.velocity);
    let len = c.design.projectile.length();
    let tt = c.design.layers[0].thickness / c.obliquity().cos();
    let dur = 1.5 * (2.0 * len + 3.0 * tt) / c.velocity;
    let st = build(&c).unwrap();
    let np = st.n_proj;
    let mut md = FemModel::new(st.mesh, st.mats, st.v0);
    md.contact = true;
    md.friction = 0.1;
    let mut sim = Fem::new(md);
    let n = sim.n_nodes();
    // nó → elementos; peça do nó (projétil se algum elemento dele é do projétil)
    let mut adj: Vec<Vec<u32>> = vec![vec![]; n];
    let mut proj = vec![false; n];
    for (e, h) in sim.mesh.hex.iter().enumerate() {
        for &k in h {
            adj[k as usize].push(e as u32);
            if sim.mesh.part[e] < np {
                proj[k as usize] = true;
            }
        }
    }
    let steps = env("STEPS", 10.0) as usize;
    println!("munição {} {:.0}° n{}  {} nós, {} elementos", a.name, c.obliquity_deg, nodes, n, sim.n_elems());
    for i in 1..=steps {
        sim.run_until(dur * i as f64 / steps as f64);
        // grade dos elementos vivos
        let h = 1.5e-3 * 2.0;
        let key = |p: [f64; 3]| p.map(|v| (v / h).floor() as i64);
        let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
        for e in 0..sim.n_elems() {
            if sim.is_alive(e) {
                let cr = sim.mesh.hex[e].map(|k| sim.x[k as usize]);
                let ec = [0, 1, 2].map(|a| cr.iter().map(|q| q[a]).sum::<f64>() / 8.0);
                grid.entry(key(ec)).or_default().push(e as u32);
            }
        }
        let (mut parts, mut bad_p, mut bad_t, mut maxd) = (0, 0, 0, 0.0f64);
        for k in 0..n {
            if adj[k].iter().any(|&e| sim.is_alive(e as usize)) {
                continue;
            }
            parts += 1;
            let p = sim.x[k];
            let kc = key(p);
            let mut hit = None;
            'o: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        if let Some(l) = grid.get(&[kc[0] + dx, kc[1] + dy, kc[2] + dz]) {
                            for &e in l {
                                let other = (sim.mesh.part[e as usize] < np) != proj[k];
                                if !other {
                                    continue;
                                }
                                let cr = sim.mesh.hex[e as usize].map(|q| sim.x[q as usize]);
                                if let Some(d) = inside(p, &cr) {
                                    let lc = (sim.mesh.corners(&sim.mesh.hex[e as usize]).iter().map(|q| q[0]).fold(f64::MIN, f64::max)
                                        - sim.mesh.corners(&sim.mesh.hex[e as usize]).iter().map(|q| q[0]).fold(f64::MAX, f64::min))
                                    .abs()
                                    .max(1e-9);
                                    hit = Some(d / lc);
                                    break 'o;
                                }
                            }
                        }
                    }
                }
            }
            if let Some(d) = hit {
                if proj[k] {
                    bad_p += 1;
                } else {
                    bad_t += 1;
                }
                maxd = maxd.max(d);
            }
        }
        println!("t={:5.1} µs  fragmentos {:5}  dentro do alvo (do projétil) {:4}  dentro do projétil (do alvo) {:4}  prof. máx {:.2} elem", dur * i as f64 / steps as f64 * 1e6, parts, bad_p, bad_t, maxd);
    }
}
