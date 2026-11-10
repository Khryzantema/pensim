use pensim_core::fem::material::lib;
use pensim_core::fem::mesh::{self, Symmetry};
use pensim_core::fem::{Fem, FemModel};
fn main() {
    let mm = 1e-3;
    let (d, t, nd) = (10.0 * mm, 5.0 * mm, 12usize);
    let r = d / 2.0;
    let dx = t / (t / (d / nd as f64)).round();
    let sym = Symmetry { y0: true, z0: true };
    let mut m = mesh::sphere(r, [-r - 0.5 * dx, 0.0, 0.0], nd, sym, 0);
    let wl = (3.0 * d / dx).round() * dx;
    let mut plate = mesh::block([0.0, -wl, -wl], [t, wl, wl], dx, sym, 1);
    for i in 0..plate.n_nodes() { let p = plate.x[i]; if p[1].abs() > wl - 1e-9 || p[2].abs() > wl - 1e-9 { plate.sym[i] |= 7; } }
    let ns = m.n_nodes();
    m.append(&plate);
    let n = m.n_nodes();
    let v: f64 = std::env::var("V").ok().and_then(|s| s.parse().ok()).unwrap_or(1000.0);
    let v0: Vec<[f64; 3]> = (0..n).map(|i| if i < ns { [v, 0.0, 0.0] } else { [0.0; 3] }).collect();
    let mut md = FemModel::new(m, vec![lib::steel_4340(), lib::al_6061_t6()], v0);
    md.contact = true;
    md.friction = 0.1;
    let mut s = Fem::new(md);
    eprintln!("KE0 = {:.1}", s.e0);
    for k in 1..=8 {
        s.run_until(k as f64 * 5e-6);
        let b = s.energy_by_part();
        eprintln!("t={:>2} µs  esfera vivo IE {:.1} HG {:.1} | erod IE {:.1} HG {:.1} || chapa vivo IE {:.1} HG {:.1} | erod IE {:.1} HG {:.1}  (KE {:.1}, contato {:.1}, vres {:.0})",
            k * 5, b[0][0].0, b[0][0].1, b[0][1].0, b[0][1].1, b[1][0].0, b[1][0].1, b[1][1].0, b[1][1].1, s.energy.kinetic, s.energy.contact, s.part_velocity(0)[0]);
    }
    eprintln!("erosão [dano, frágil, distorção, volume] = {:?}", s.erosion_reasons());
}
