use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let name = std::env::var("AMMO").unwrap();
    let a = AMMO.iter().find(|a| a.name.contains(&name)).unwrap();
    let s = env("NODES", 8.0) * 1.5e-3 / (a.caliber_mm * 1e-3);
    let mut c = a.config(s);
    c.design.layers[0].thickness = env("PLATE", 20.0) * 1e-3 * s;
    adjust_rha(&mut c.design.layers[0].material, env("PLATE", 20.0) * 1e-3);
    c.obliquity_deg = env("A", 0.0);
    c.velocity = env("V", c.velocity);
    c.n_frames = 12;
    let mut sim = Sim::new(c).unwrap();
    sim.run_to_end();
    let o = sim.outcome();
    let ids = sim.part_ids();
    let fr = sim.frames.last().unwrap().unpack();
    let (mut tot, mut bad) = (0, 0);
    for (i, &id) in ids.iter().enumerate() { if id < 2 { tot += 1; if fr.damage[i] >= 0.6 { bad += 1; } } }
    println!("{:?} proj nós danificados {bad}/{tot} = {:.0}%", o.kind, 100.0 * bad as f64 / tot.max(1) as f64);
}
