use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let a = AMMO.iter().find(|a| a.name.contains("128 mm")).unwrap();
    let s = env("S", 0.1);
    let mut c = a.config(s);
    c.design.layers[0].thickness = env("PLATE", 300.0) * 1e-3 * s;
    let w = env("WD", 7.0) * a.caliber_mm * 1e-3 * s;
    c.design.layers[0].width = w; c.design.layers[0].height = w;
    c.dx = env("DX", 1.5) * 1e-3;
    c.horizon_m = env("HZ", 2.5);
    c.sleep_ratio = env("SLEEP", 0.03);
    c.symmetry = env("SYM", 1.0) > 0.5;
    c.n_frames = 20;
    let est = c.estimate_nodes();
    let d_nodes = a.caliber_mm * 1e-3 * s / c.dx;
    println!("escala {:.3}: Ø{:.1}mm ({:.1} nós no diâmetro) placa {:.1}mm largura {:.0}mm dx {:.2}mm · nós físicos ≈ {:.0}k · mem ≈ {:.0} MB",
        s, a.caliber_mm*s, d_nodes, c.design.layers[0].thickness*1e3, w*1e3, c.dx*1e3, est/1e3, c.estimate_memory_mb());
    let t = std::time::Instant::now();
    let mut sim = Sim::new(c).unwrap();
    let setup = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    sim.run_to_end();
    let run = t.elapsed().as_secs_f64();
    let o = sim.outcome();
    println!("  calc={} nós · dt={:.2e} · passos={} · setup {:.2}s · run {:.1}s ({:.2} ms/passo) · acordados {:.0}% · {:?} pen={:.1}mm absorvida={:.0}% err={:.1}%",
        sim.n(), sim.dt, sim.step_count, setup, run, 1e3*run/sim.step_count as f64, 100.0*sim.awake_fraction(), o.kind, o.penetration*1e3, o.energy_absorbed_pct, o.energy_error_pct);
}
