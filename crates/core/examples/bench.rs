use pensim_core::*;
fn main() {
    let mut c = Preset::Rifle762.config();
    c.velocity = 900.0;
    c.n_frames = 6;
    let t = std::time::Instant::now();
    let mut s = Sim::new(c).unwrap();
    let setup = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    s.run_to_end();
    let run = t.elapsed().as_secs_f64();
    let o = s.outcome();
    println!("nós={} dt={:.3e} passos={} setup={setup:.2}s run={run:.2}s ({:.2} ms/passo) {:?} vres={:.0} err={:.2}%", s.n(), s.dt, s.step_count, 1e3*run/s.step_count as f64, o.kind, o.v_residual, o.energy_error_pct);
}
