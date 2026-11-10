//! Caso de calibração esfera × chapa: roda no PenSim e escreve os decks do OpenCourant com a
//! mesma malha. Variáveis: V (m/s), T (mm), A (graus), ND (elementos no diâmetro), MAT (al|rha),
//! OUT (pasta), RUN=0 para só escrever os decks.
use pensim_core::fem::material::{from_library, lib, FemMat};
use pensim_core::fem::mesh::{self, Symmetry};
use pensim_core::fem::radioss;
use pensim_core::fem::{Fem, FemModel};

fn env(k: &str, d: f64) -> f64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn main() {
    let mm = 1e-3;
    let v = env("V", 1000.0);
    let t = env("T", 5.0) * mm;
    let ang = env("A", 0.0);
    let nd = env("ND", 12.0) as usize;
    let dur = env("DUR", 40.0) * 1e-6;
    let plate_mat: FemMat = match std::env::var("MAT").unwrap_or("al".into()).as_str() {
        "rha" => {
            let mut m = pensim_core::LIBRARY[0].clone();
            pensim_core::adjust_rha(&mut m, t);
            from_library(&m)
        }
        _ => lib::al_6061_t6(),
    };
    let out = std::env::var("OUT").unwrap_or("/tmp/calib".into());
    std::fs::create_dir_all(&out).unwrap();
    let d = 10.0 * mm;
    let r = d / 2.0;
    let dx = t / (t / (d / nd as f64)).round().max(2.0);
    let sym = if ang == 0.0 { Symmetry { y0: true, z0: true } } else { Symmetry { y0: false, z0: true } };
    let cosa = ang.to_radians().cos();
    let mut m = mesh::sphere(r, [-(r + 0.5 * dx) / cosa, 0.0, 0.0], nd, sym, 0);
    let wl = (3.0 * d / dx).round() * dx;
    let mut plate = mesh::block([0.0, -wl, -wl], [t, wl, wl], dx, sym, 1);
    for i in 0..plate.n_nodes() {
        let p = plate.x[i];
        if p[1].abs() > wl - 1e-9 || p[2].abs() > wl - 1e-9 {
            plate.sym[i] |= 7;
        }
    }
    if ang != 0.0 {
        let (s, c) = ang.to_radians().sin_cos();
        for p in &mut plate.x {
            let (x, y) = (p[0], p[1]);
            p[0] = c * x - s * y;
            p[1] = s * x + c * y;
        }
    }
    let ns = m.n_nodes();
    m.append(&plate);
    m.hex = m.hex.iter().map(|h| mesh::orient(&m.x, *h)).collect();
    let n = m.n_nodes();
    let v0: Vec<[f64; 3]> = (0..n).map(|i| if i < ns { [v, 0.0, 0.0] } else { [0.0; 3] }).collect();
    let mats = vec![lib::steel_4340(), plate_mat];
    let name = format!("esf_{}_{:.1}mm_{:.0}g_{:.0}ms", std::env::var("MAT").unwrap_or("al".into()), t / mm, ang, v);
    let (st, en) = radioss::write(&radioss::DeckInput { name: &name, mesh: &m, mats: &mats, v0: &v0, friction: 0.1, t_end: dur, th_dt: 0.5e-6, isolid: env("ISOLID", 24.0) as i32 });
    std::fs::write(format!("{out}/{name}_0000.rad"), st).unwrap();
    std::fs::write(format!("{out}/{name}_0001.rad"), en).unwrap();
    println!("NAME {name}");
    println!("deck {out}/{name}_0000.rad ({} nós, {} elementos)", n, m.n_elems());
    if env("RUN", 1.0) < 0.5 {
        return;
    }
    let mut md = FemModel::new(m, mats, v0);
    md.contact = true;
    md.friction = 0.1;
    let t0 = std::time::Instant::now();
    let mut sim = Fem::new(md);
    sim.run_until(dur);
    let vp = sim.part_velocity(0);
    println!("RESULT {} {:.3} {:.3} {:.3} {} {:.4} {:.2}", name, vp[0], vp[1], vp[2], sim.n_eroded, 100.0 * sim.energy.balance(sim.e0) / sim.e0, t0.elapsed().as_secs_f64());
    println!("PenSim: v_cg esfera = ({:.1}, {:.1}, {:.1}) m/s, |v| = {:.1}, erodidos {}, energia {:.2}%, {:.1} s", vp[0], vp[1], vp[2], (vp[0] * vp[0] + vp[1] * vp[1] + vp[2] * vp[2]).sqrt(), sim.n_eroded, 100.0 * sim.energy.balance(sim.e0) / sim.e0, t0.elapsed().as_secs_f64());
}
