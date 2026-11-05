use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let mut c = Preset::Rifle762.config();
    c.dx = 0.0015; c.n_frames = 20;
    c.velocity = env("V", 850.0);
    c.obliquity_deg = env("A", 80.0);
    let t_end0 = c.t_end();
    let mut s = Sim::new(c).unwrap();
    let (vx0, vz0) = (0.0, 0.0);
    let _ = (vx0, vz0);
    let t = std::time::Instant::now();
    s.run_to_end();
    let o = s.outcome();
    println!("α={}°  v={} → {:?} | t_sim={:.0}µs de t_end(máx)={:.0}µs  separação={:?}µs  v_res={:.0} m/s  pen={:.1}mm  abs={:.0}%  {:.1}s",
        env("A",80.0), env("V",850.0), o.kind, s.time*1e6, t_end0*1e6, s.separation_time().map(|t| (t*1e6).round()), o.v_residual, o.penetration*1e3, o.energy_absorbed_pct, t.elapsed().as_secs_f64());
}
