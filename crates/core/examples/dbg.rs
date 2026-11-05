use pensim_core::*;
fn main() {
    let mut c = Preset::Rifle762.config();
    c.dx = 0.002;
    for l in &mut c.design.layers { l.width = 0.04; l.height = 0.04; l.thickness = 0.014; }
    c.velocity = 1500.0; c.n_frames = 10;
    let full = c.t_end();
    let mut s = Sim::new(c).unwrap();
    s.run_to_end();
    let o = s.outcome();
    println!("t_end original={:.1}us parou={:.1}us {:?} behind={:.0}% vres={:.0}", full*1e6, s.time*1e6, o.kind, o.proj_behind_pct, o.v_residual);
}
