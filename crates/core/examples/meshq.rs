use pensim_core::fem::mesh::{self, Symmetry};
fn main() {
    for nd in [8usize, 10, 16] {
        for (name, m, dx) in [("cil", mesh::cylinder_x(1.0, 0.0, 2.0, nd, Symmetry::default(), 0), 2.0 / nd as f64), ("esf", mesh::sphere(1.0, [0.0; 3], nd, Symmetry::default(), 0), 2.0 / nd as f64)] {
            let nom = (dx / 2.0f64).powi(3);
            let mut worst = (f64::MAX, 0usize);
            for (e, h) in m.hex.iter().enumerate() {
                let j = mesh::corner_jacobian_min(&m.corners(h)) / nom;
                if j < worst.0 { worst = (j, e); }
            }
            let c = m.corners(&m.hex[worst.1]);
            let cen: Vec<f64> = (0..3).map(|a| c.iter().map(|p| p[a]).sum::<f64>() / 8.0).collect();
            let vol = mesh::hex_volume(&c) / (dx * dx * dx);
            eprintln!("{name} nd={nd}: jac. escalonado mín {:.3}; pior {:.3} no elemento em {:?} (vol/nominal {:.2})", m.min_scaled_jacobian(), worst.0, cen.iter().map(|v| (v * 100.0).round() / 100.0).collect::<Vec<_>>(), vol);
        }
    }
}
