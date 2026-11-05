use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let mut c = Preset::Rifle762.config();
    c.dx = 0.001;
    c.velocity = env("V", 1200.0);
    let w = env("W", 14.0) * 1e-3;
    c.design.layers[0].width = w;
    c.design.layers[0].height = w;
    c.design.layers[0].thickness = 0.008;
    c.clamp_edges = env("CLAMP", 1.0) > 0.5;
    c.n_frames = 4;
    let mut s = Sim::new(c).unwrap();
    s.run_to_end();
    let o = s.outcome();
    println!("largura {:.0} mm clamp={} → {:?} v_res={:.0} ejetado={:.1}% abs={:.0}% err={:.2}%", w * 1e3, env("CLAMP", 1.0) > 0.5, o.kind, o.v_residual, o.target_ejected_pct, o.energy_absorbed_pct, o.energy_error_pct);
}
