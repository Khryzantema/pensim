use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let name = std::env::var("AMMO").unwrap_or("PzGr 39/42".into());
    let a = AMMO.iter().find(|a| a.name.contains(&name)).unwrap();
    let s = 8.0 * 1.5e-3 / (a.caliber_mm * 1e-3);
    let mut c = a.config(s);
    let pl = env("PLATE", 100.0);
    c.design.layers[0].thickness = pl * 1e-3 * s;
    adjust_rha(&mut c.design.layers[0].material, pl * 1e-3);
    c.n_frames = 10;
    let mut sim = Sim::new(c).unwrap();
    while !sim.done() {
        sim.advance_for(std::time::Duration::from_millis(400));
        let o = sim.outcome();
        println!("t={:>6.0}µs(real) v_res={:>5.0} atrás={:>3.0}% {:?}", sim.time * 1e6 / s, o.v_residual, o.proj_behind_pct, o.kind);
    }
}
