use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let a = AMMO.iter().find(|a| a.name.contains("Mango")).unwrap();
    let s = env("S", a.scale);
    let mut c = a.config(s);
    c.design.layers[0].thickness = env("PLATE", 300.0) * 1e-3 * s;
    adjust_rha(&mut c.design.layers[0].material, env("PLATE", 300.0) * 1e-3);
    c.dx = env("DX", 1.5) * 1e-3;
    c.n_frames = 30;
    c.rate_gain = env("RG", 1.0);
    let l = c.design.projectile.length();
    println!("escala {s:.2}: dardo Ø{:.1}mm L={:.0}mm ({:.1} nós no Ø) placa {:.0}mm (real {:.0}mm) v={}", c.design.projectile.max_radius()*2e3, l*1e3, c.design.projectile.max_radius()*2.0/c.dx, c.design.layers[0].thickness*1e3, env("PLATE",300.0), c.velocity);
    let mut sim = Sim::new(c).unwrap();
    let t = std::time::Instant::now();
    sim.run_to_end();
    let o = sim.outcome();
    let frac = o.penetration / (sim.plate_back());
    println!("  nós calc={} passos={} t_sim={:.1}µs (t_end máx {:.1}µs) run={:.0}s → {:?} ponta atingiu x={:.1}mm = {:.0}% da espessura, v_res={:.0} absorvida={:.0}% sep={:?}",
        sim.n(), sim.step_count, sim.time*1e6, sim.t_end()*1e6, t.elapsed().as_secs_f64(), o.kind, o.penetration*1e3, 100.0*frac, o.v_residual, o.energy_absorbed_pct, sim.separation_time().map(|t| t*1e6));
}
