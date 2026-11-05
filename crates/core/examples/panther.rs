use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let a = AMMO.iter().find(|a| a.name.contains("75 mm")).unwrap();
    let nodes = env("NODES", 8.0);
    // escala interna como no app: nós no diâmetro × dx / diâmetro real
    let s = nodes * 1.5e-3 / (a.caliber_mm * 1e-3);
    let mut c = a.config(s);
    c.design.layers[0].thickness = env("PLATE", 140.0) * 1e-3 * s;
    if env("CORE", 1.0) < 0.5 { c.design.projectile.core = None; }
    c.n_frames = 20;
    c.sleep_ratio = env("SLEEP", 0.03);
    c.symmetry = env("SYM", 1.0) > 0.5;
    c.noise = env("NOISE", 0.15);
    c.deviatoric_plasticity = env("DEV", 1.0) > 0.5;
    c.rate_gain = env("RG", 1.0);
    for l in c.design.layers.iter_mut() { l.material.comp_factor = env("COMP", l.material.comp_factor); }
    let py = env("PY", c.design.projectile.material.yield_strength / 1e9) * 1e9;
    c.design.projectile.material.yield_strength = py;
    c.design.projectile.material.fail_stretch = env("PF", c.design.projectile.material.fail_stretch);
    let m = c.design.projectile.material.name;
    let t = std::time::Instant::now();
    let mut sim = Sim::new(c).unwrap();
    sim.run_to_end();
    let o = sim.outcome();
    // lábio de entrada: nós do alvo empurrados para fora da face frontal (x < 0)
    let ids = sim.part_ids();
    let fr = sim.frames.last().unwrap().unpack();
    let np = 2;
    let lip = fr.pos.iter().zip(&ids).filter(|(_, &id)| id >= np).map(|(p, _)| p[0]).fold(f32::MAX, f32::min);
    println!("  lábio (altura máx. acima da face original) = {:.1} mm reais", (-lip as f64).max(0.0) * 1e3 / s);
    println!("  fim: t_sim={:.0}µs  separação={:?}µs  t_end máx={:.0}µs", sim.time*1e6, sim.separation_time().map(|t| (t*1e6).round()), sim.t_end()*1e6);
    println!("{m} | núcleo={} nós={} sleep={} sym={} → {:?} ponta={:.0} mm reais ({:.0}% de {}) v_res={:.0} abs={:.0}% {:.0}s",
        env("CORE",1.0) > 0.5, nodes, env("SLEEP",0.03), env("SYM",1.0)>0.5, o.kind, o.penetration*1e3/s, 100.0*o.penetration*1e3/s/env("PLATE",140.0), env("PLATE",140.0), o.v_residual, o.energy_absorbed_pct, t.elapsed().as_secs_f64());
}
