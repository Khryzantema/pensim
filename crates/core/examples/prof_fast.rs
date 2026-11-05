use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let a = AMMO.iter().find(|a| a.name.contains("Mango")).unwrap();
    let s = a.scale;
    let mut c = a.config(s);
    c.design.layers[0].thickness = env("PLATE", 300.0) * 1e-3 * s;
    c.dx = env("DX", 1.0) * 1e-3;
    c.n_frames = 30;
    c.state_based = env("OSB", 1.0) > 0.5;
    c.sleep_ratio = env("SLEEP", c.sleep_ratio);
    let mut sim = Sim::new(c).unwrap();
    println!("nós {} dt={:.3e}", sim.n(), sim.dt);
    let t0 = std::time::Instant::now();
    let mut last = t0.elapsed().as_secs_f64();
    while !sim.done() {
        let s0 = sim.step_count;
        let a = std::time::Instant::now();
        sim.advance_for(std::time::Duration::from_secs(5));
        let k = sim.step_count - s0;
        println!("t={:.1}µs passos={} awake={:.1}% {:.1} ms/passo", sim.time * 1e6, sim.step_count, 100.0 * sim.awake_fraction(), a.elapsed().as_secs_f64() * 1e3 / k.max(1) as f64);
    }
    let k = sim.step_count;
    println!("t_end={:.1}µs sep={:?} {:?}", sim.t_end()*1e6, sim.separation_time().map(|t| t*1e6), sim.outcome().kind);
    let o = sim.outcome(); println!("v_res={:.0} pen={:.1}mm abs={:.0}% err={:.2}%", o.v_residual, o.penetration*1e3, o.energy_absorbed_pct, o.energy_error_pct);
    println!("fases ms/passo: grade {:.2} forças {:.2} (passo1 {:.2}) integração {:.2}", 1e3*sim.prof[0]/sim.step_count as f64, 1e3*sim.prof[1]/sim.step_count as f64, 1e3*sim.prof_p1/sim.step_count as f64, 1e3*sim.prof[2]/sim.step_count as f64);
    println!("total {:.1}s passos {k}", t0.elapsed().as_secs_f64());
}
