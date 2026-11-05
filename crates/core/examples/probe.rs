use pensim_core::*;
fn main() {
    let base = Preset::Rifle762.config();
    for v in [400.0, 700.0, 900.0, 1200.0] {
        let mut c = base.clone(); c.velocity = v; c.n_frames = 4;
        let t = std::time::Instant::now();
        let mut s = Sim::new(c).unwrap();
        let (n, dt) = (s.n(), s.dt);
        s.run_to_end();
        let o = s.outcome();
        println!("v={v} n={n} dt={dt:.2e} steps={} {:?} vres={:.0} abs={:.0}% ej={:.1}% err={:.1}% {:.1}s",
          s.step_count, o.kind, o.v_residual, o.energy_absorbed_pct, o.target_ejected_pct, o.energy_error_pct, t.elapsed().as_secs_f64());
    }
}
