use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let name = std::env::var("AMMO").unwrap_or("7,62".into());
    let a = AMMO.iter().find(|a| a.name.contains(&name)).unwrap_or(&AMMO[0]);
    let nodes = env("NODES", 16.0);
    let s = nodes * 1.5e-3 / (a.caliber_mm * 1e-3);
    let mut c = a.config(s);
    c.dx = 1.5e-3;
    c.duration = env("DUR", 1000.0) * 1e-6 * s;
    c.obliquity_deg = env("A", 0.0);
    let t = std::time::Instant::now();
    let mut sim = fem::run::FemSim::new(c).unwrap();
    eprintln!("{}: {} nós, {} elementos, montagem {:.1} s", a.name, sim.n_physical(), sim.hex().len(), t.elapsed().as_secs_f64());
    let mut last = 0.0;
    while !sim.done() {
        sim.advance_for(std::time::Duration::from_millis(500));
        let p = sim.progress();
        if p - last > 0.1 { last = p; let o = sim.outcome(); eprintln!("  {:.0}% em {:.1} s (passo {}, dt {:.2e}) v_res {:.0} {:?}", p * 100.0, t.elapsed().as_secs_f64(), sim.steps(), sim.dt(), o.v_residual, o.kind); }
    }
    let o = sim.outcome();
    let sz: usize = sim.frames.iter().map(|f| f.size_bytes()).sum();
    eprintln!("fim {:.1} s: {:?} v_res {:.0} m/s, energia {:.2}%, erodidos {}, {} quadros ({:.1} MB)", t.elapsed().as_secs_f64(), o.kind, o.v_residual, o.energy_error_pct, sim.fem.n_eroded, sim.frames.len(), sz as f64 / 1e6);
}
