//! Confere a massa dos projéteis montados contra a massa real (validação das dimensões).
use pensim_core::*;
fn main() {
    let res: f64 = std::env::var("RES").ok().and_then(|v| v.parse().ok()).unwrap_or(24.0);
    for a in AMMO {
        let mut c = a.config(1.0);
        c.dx = a.caliber_mm * 1e-3 / res;
        c.jitter = 0.0;
        let m = Model::build_full(&c).unwrap();
        let mats = c.design.materials();
        let vol = c.dx.powi(3);
        let mut per = vec![0.0f64; mats.len()];
        for &p in &m.part {
            if (p as usize) < c.design.n_proj_parts() {
                per[p as usize] += mats[p as usize].rho * vol;
            }
        }
        let tot: f64 = per.iter().sum::<f64>() * 1e3;
        let parts: Vec<String> = (0..c.design.n_proj_parts()).filter(|&i| per[i] > 0.0).map(|i| format!("{} {:.0}%", mats[i].name, 100.0 * per[i] * 1e3 / tot)).collect();
        println!("{:<48} modelo {:>8.1} g  real {:>8.1} g  ({:+.0}%)  | {}", a.name, tot, a.mass_g, 100.0 * (tot / a.mass_g - 1.0), parts.join(", "));
    }
}
