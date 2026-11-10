use pensim_core::fem::build::build;
use pensim_core::fem::mesh;
use pensim_core::*;
fn main() {
    let key = std::env::var("AMMO").unwrap_or(".50".into());
    let a = AMMO.iter().find(|a| a.name.contains(&key)).unwrap();
    let nodes = 12.0;
    let s = nodes * 1.5e-3 / (a.caliber_mm * 1e-3);
    let mut c = a.config(s);
    c.dx = 1.5e-3;
    let st = build(&c).unwrap();
    let m = &st.mesh;
    let mut worst: Vec<(f64, usize)> = m.hex.iter().enumerate().map(|(e, h)| (mesh::scaled_jacobian_min(&m.corners(h)), e)).collect();
    worst.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(j, e) in worst.iter().take(8) {
        let c = m.corners(&m.hex[e]);
        let cen: Vec<f64> = (0..3).map(|k| c.iter().map(|p| p[k]).sum::<f64>() / 8.0 * 1e3).collect();
        println!("elem {} peça {} jac.esc {:.3} vol {:.3e} centro {:.1?} mm", e + 1, m.part[e], j, mesh::hex_volume(&c), cen);
    }
    let bad = worst.iter().filter(|w| w.0 < 0.2).count();
    println!("{} elementos, {} com jacobiano escalonado < 0,2, {} < 0", m.n_elems(), bad, worst.iter().filter(|w| w.0 <= 0.0).count());
}
