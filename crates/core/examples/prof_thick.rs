use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let a = AMMO.iter().find(|a| a.name.contains("Mango")).unwrap();
    let s = a.scale;
    let mut c = a.config(s);
    c.design.layers[0].thickness = env("PLATE", 800.0) * 1e-3 * s;
    c.dx = env("DX", 1.5) * 1e-3;
    c.n_frames = 30;
    c.sleep_ratio = env("SLEEP", 0.03);
    let tl = env("TLIM", 400.0) * 1e-6;
    let mut sim = Sim::new(c).unwrap();
    println!("nós calc={} dt={:.2e}", sim.n(), sim.dt);
    let (mut last_t, mut last_steps, mut wall0) = (0.0, 0u64, std::time::Instant::now());
    let t0 = std::time::Instant::now();
    while sim.time < tl && !sim.done() {
        sim.advance_for(std::time::Duration::from_millis(200));
        if sim.time - last_t >= 50e-6 {
            let st = sim.step_count - last_steps;
            let ms = 1e3 * wall0.elapsed().as_secs_f64() / st.max(1) as f64;
            println!("t={:>6.0}µs  acordados={:>3.0}%  {:>6.2} ms/passo  (progresso {:.0}%)", sim.time*1e6, 100.0*sim.awake_fraction(), ms, 100.0*sim.progress());
            last_t = sim.time; last_steps = sim.step_count; wall0 = std::time::Instant::now();
        }
    }
    println!("total {:.1}s, {} passos", t0.elapsed().as_secs_f64(), sim.step_count);
}
