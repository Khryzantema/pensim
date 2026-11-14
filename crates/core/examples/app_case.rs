//! Mesma configuração que o app monta (load_ammo(k, 1.0) + scaled_cfg): AMMO (índice), NODES,
//! DUR (µs reais). Mostra a maior velocidade de nó a cada passo de tempo de relatório.
use pensim_core::*;
fn env(k: &str, d: f64) -> f64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}
fn main() {
    let a = &AMMO[env("AMMO", 8.0) as usize];
    let mut c = a.config(1.0);
    let d = c.design.projectile.max_radius() * 2.0;
    let f = (env("NODES", 12.0) * c.dx / d).clamp(0.004, 6.0);
    c.duration = env("DUR", 150.0) * 1e-6 * f;
    for l in c.design.layers.iter_mut() {
        adjust_rha(&mut l.material, l.thickness);
    }
    c.design.projectile.scale(f);
    for l in c.design.layers.iter_mut() {
        l.thickness *= f;
        l.gap_before *= f;
        l.width *= f;
        l.height *= f;
    }
    let mut sim = fem::run::FemSim::new(c).unwrap();
    eprintln!("{}: escala {:.3}, {} elementos", a.name, f, sim.hex().len());
    let t = std::time::Instant::now();
    let mut last = -1.0;
    while !sim.done() {
        sim.advance_for(std::time::Duration::from_millis(200));
        let tm = sim.time() / f * 1e6;
        if tm - last >= env("EVERY", 2.0) || sim.done() {
            last = tm;
            let (vm, im) = sim.fem.v.iter().enumerate().map(|(i, v)| ((v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt(), i)).fold((0.0, 0), |a, b| if b.0 > a.0 { b } else { a });
            let o = sim.outcome();
            let live = sim.fem.mesh.hex.iter().enumerate().filter(|(e, h)| h.contains(&(im as u32)) && sim.fem.is_alive(*e)).count();
            let tot = sim.fem.mesh.hex.iter().filter(|h| h.contains(&(im as u32))).count();
            let nz = (0..sim.fem.n_nodes()).filter(|&i| sim.fem.node_mass(i) <= 0.0).count();
            eprintln!("      nó {im}: {live}/{tot} elementos vivos; incorporados até agora: {nz}");
            eprintln!("t={:6.1} µs  vmax {:10.0} nó {:6} massa {:.2e}  erodidos {}  energia {:.2}%  passo {:.2e}  ({:.0} s)", tm, vm, im, sim.fem.node_mass(im), sim.fem.n_eroded, o.energy_error_pct, sim.dt(), t.elapsed().as_secs_f64());
        }
    }
}
