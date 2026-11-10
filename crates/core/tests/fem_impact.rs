//! Fase 2: contato + erosão. Esfera de aço contra chapa de alumínio (docs/NOVO-NUCLEO.md).
use pensim_core::fem::material::lib;
use pensim_core::fem::mesh::{self, Symmetry};
use pensim_core::fem::{Fem, FemModel};

const MM: f64 = 1e-3;

pub struct Shot {
    pub v_res: f64,
    pub x_cg: f64,
    pub balance: f64,
    pub hg_ratio: f64,
    /// hourglass/interna só dos elementos vivos
    pub hg_live: f64,
    pub eroded: usize,
    pub elems: usize,
    pub steps: u64,
    pub secs: f64,
}

pub fn sphere_vs_plate(v: f64, t_plate: f64, angle_deg: f64, nd: usize, sym: Symmetry, t_end: f64) -> Shot {
    let d = 10.0 * MM;
    let r = d / 2.0;
    let dx = t_plate / (t_plate / (d / nd as f64)).round().max(2.0);
    let w = 3.0 * d;
    // esfera fora da chapa com folga de meio elemento (medida na normal da chapa), voando em x
    let cosa = angle_deg.to_radians().cos();
    let mut m = mesh::sphere(r, [-(r + 0.5 * dx) / cosa, 0.0, 0.0], nd, sym, 0);
    let wl = (w / dx).round() * dx;
    let mut plate = mesh::block([0.0, -wl, -wl], [t_plate, wl, wl], dx, sym, 1);
    // borda presa
    for i in 0..plate.n_nodes() {
        let p = plate.x[i];
        if p[1].abs() > wl - 1e-9 || p[2].abs() > wl - 1e-9 {
            plate.sym[i] |= 7;
        }
    }
    // chapa inclinada em torno de z (plano de simetria y = 0 continua válido só sem simetria em y)
    if angle_deg != 0.0 {
        let (s, c) = angle_deg.to_radians().sin_cos();
        for p in &mut plate.x {
            let (x, y) = (p[0], p[1]);
            p[0] = c * x - s * y;
            p[1] = s * x + c * y;
        }
    }
    let n_sphere = m.n_nodes();
    m.append(&plate);
    let n = m.n_nodes();
    let ne = m.n_elems();
    let v0: Vec<[f64; 3]> = (0..n).map(|i| if i < n_sphere { [v, 0.0, 0.0] } else { [0.0; 3] }).collect();
    let mut md = FemModel::new(m, vec![lib::steel_4340(), lib::al_6061_t6()], v0);
    md.contact = true;
    md.friction = 0.1;
    let t0 = std::time::Instant::now();
    let mut sim = Fem::new(md);
    let e0 = sim.e0;
    sim.run_until(t_end);
    let en = sim.energy;
    let by = sim.energy_by_part();
    let (ie_l, hg_l) = by.iter().fold((0.0, 0.0), |a, p| (a.0 + p[0].0, a.1 + p[0].1));
    Shot {
        hg_live: hg_l / ie_l.max(1e-30),
        v_res: sim.part_velocity(0)[0],
        x_cg: sim.part_center(0)[0],
        balance: en.balance(e0) / e0,
        hg_ratio: en.hourglass / en.internal.max(1e-30),
        eroded: sim.n_eroded,
        elems: ne,
        steps: sim.steps,
        secs: t0.elapsed().as_secs_f64(),
    }
}

fn show(name: &str, s: &Shot) {
    eprintln!(
        "{name}: {} elementos, {} passos em {:.1} s; v_res = {:.0} m/s, cg x = {:.1} mm, {} erodidos, erro de energia {:.2}%, HG/IE vivos {:.1}% (com erodidos {:.1}%)",
        s.elems, s.steps, s.secs, s.v_res, s.x_cg / MM, s.eroded, s.balance * 100.0, s.hg_live * 100.0, s.hg_ratio * 100.0
    );
}

#[test]
fn sphere_perforates_plate_symmetric() {
    let q = sphere_vs_plate(1000.0, 5.0 * MM, 0.0, 12, Symmetry { y0: true, z0: true }, 40e-6);
    show("¼", &q);
    let h = sphere_vs_plate(1000.0, 5.0 * MM, 0.0, 12, Symmetry { y0: true, z0: false }, 40e-6);
    show("½", &h);
    for s in [&q, &h] {
        assert!(s.v_res > 0.0 && s.x_cg > 5.0 * MM, "tinha de perfurar");
        assert!(s.balance.abs() < 0.05, "energia");
        assert!(s.hg_live < 0.1, "hourglass");
    }
    assert!((q.v_res / h.v_res - 1.0).abs() < 0.02, "simetria: {} × {}", q.v_res, h.v_res);
}

/// Convergência rápida (não é teste: `cargo test -- --ignored convergence`).
#[test]
#[ignore]
fn convergence() {
    for nd in [8, 12, 16, 24] {
        let s = sphere_vs_plate(1000.0, 5.0 * MM, 0.0, nd, Symmetry { y0: true, z0: true }, 40e-6);
        show(&format!("{nd} no diâmetro"), &s);
    }
}

/// Chapa inclinada (giro em z; o plano z = 0 continua de simetria).
#[test]
#[ignore]
fn oblique() {
    for ang in [0.0, 45.0, 60.0, 70.0] {
        let s = sphere_vs_plate(1000.0, 5.0 * MM, ang, 12, Symmetry { y0: false, z0: true }, 50e-6);
        show(&format!("{ang}°"), &s);
    }
}
