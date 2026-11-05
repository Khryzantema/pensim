use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let mut c = Preset::Rifle762.config();
    c.velocity = env("V", 400.0);
    c.dx = 0.0015;
    c.n_frames = 20;
    c.state_based = env("OSB", 1.0) > 0.5;
    c.sleep_ratio = env("SLEEP", 0.05);
    c.rate_gain = env("RG", 1.0);
    c.symmetry = env("SYM", 1.0) > 0.5;
    c.noise = env("NOISE", c.noise);
    if env("NOFAILT", 0.0) > 0.5 { for l in c.design.layers.iter_mut() { l.material.fail_stretch = 10.0; } }
    if env("NOFAILP", 0.0) > 0.5 { c.design.projectile.material.fail_stretch = 10.0; }
    if env("NOFAIL", 0.0) > 0.5 { for l in c.design.layers.iter_mut() { l.material.fail_stretch = 10.0; } c.design.projectile.material.fail_stretch = 10.0; }
    let mut s = Sim::new(c).unwrap();
    while !s.done() {
        s.advance_for(std::time::Duration::from_millis(30));
        if let Some(f) = s.frames.last() {
            let o = s.outcome();
            if std::env::var("TOP").is_ok() && s.done() {
                let fr = f.unpack();
                let ids = s.part_ids();
                let mut v: Vec<(f32, usize)> = fr.speed.iter().cloned().zip(0..).collect();
                v.sort_by(|a, b| b.0.total_cmp(&a.0));
                for &(sp, i) in v.iter().take(12) {
                    println!("  nó {i} parte {} |v|={sp:.0} dano={:.2} εp={:.2} T={:.0} pos=({:.1},{:.1},{:.1})mm", ids[i], fr.damage[i], fr.plastic[i], fr.temp[i], fr.pos[i][0]*1e3, fr.pos[i][1]*1e3, fr.pos[i][2]*1e3);
                }
            }
            println!("t={:.1}µs passos={} KEp={:.2} KEt={:.2} el={:.2} pl={:.2} fr={:.2} ct={:.2} err={:.1}%", f.time * 1e6, s.step_count, f.ke_proj, f.ke_target, f.e_elastic, f.e_plastic, f.e_fracture, f.e_contact, o.energy_error_pct);
        }
    }
}
// (debug) nós mais rápidos no fim
#[allow(dead_code)]
fn top(s: &Sim) {}
