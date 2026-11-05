use pensim_core::*;
fn main() {
    let mut c = Preset::Rifle762.config();
    c.dx = 0.002;
    for l in &mut c.design.layers { l.width = 0.04; l.height = 0.04; l.thickness = 0.014; }
    c.velocity = 1500.0;
    c.n_frames = 10;
    let mut s = Sim::new(c).unwrap();
    s.run_to_end();
    let o = s.outcome();
    println!("{:?} t={:.1} t_end={:.1} sep={:?} vres={:.0} atrás={:.0}%", o.kind, s.time * 1e6, s.t_end() * 1e6, s.separation_time().map(|t| t * 1e6), o.v_residual, o.proj_behind_pct);
}
