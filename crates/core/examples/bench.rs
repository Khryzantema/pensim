use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let sym = env("SYM", 1.0) > 0.5;
    let mut c = Preset::Rifle762.config();
    c.velocity = env("V", 900.0); c.n_frames = 6; c.horizon_m = env("HZ", 2.5);
    c.obliquity_deg = env("A", 0.0); c.symmetry = sym; c.dx = env("DX", 1.5) * 1e-3;
    let t = std::time::Instant::now();
    let mut s = Sim::new(c).unwrap();
    let setup = t.elapsed().as_secs_f64();
    let t = std::time::Instant::now();
    s.run_to_end();
    let run = t.elapsed().as_secs_f64();
    let o = s.outcome();
    let f = s.frames.last().unwrap();
    let vm = f.von_mises.iter().cloned().fold(0.0f32, f32::max);
    println!("fases ms/passo: grade {:.2} forças {:.2} integração {:.2}", 1e3*s.prof[0]/s.step_count as f64, 1e3*s.prof[1]/s.step_count as f64, 1e3*s.prof[2]/s.step_count as f64);
    println!("sym={} máscara={} nós calc={} físicos={} dt={:.3e} passos={} setup={setup:.2}s run={run:.2}s ({:.2} ms/passo) {:?} vres={:.1} abs={:.1}% ej={:.1}% err={:.2}% vm_max={:.2}GPa Eplast={:.3}J",
        sym as u8, s.symmetry(), s.n(), s.n_physical(), s.dt, s.step_count, 1e3*run/s.step_count as f64, o.kind, o.v_residual, o.energy_absorbed_pct, o.target_ejected_pct, o.energy_error_pct, vm as f64*1e-9, f.e_plastic);
}
