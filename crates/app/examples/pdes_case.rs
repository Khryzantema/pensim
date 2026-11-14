//! Roda um projétil desenhado (.pdes) como o app: PDES=arquivo, NODES, DUR (µs reais), A.
use pensim_core::*;
fn env(k: &str, d: f64) -> f64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}
fn main() {
    let path = std::env::var("PDES").expect("PDES");
    let d = pensim_app::draw::Drawing::from_text(&std::fs::read_to_string(&path).unwrap()).expect("desenho");
    let p = d.to_projectile(1.0).expect("projétil");
    let mut c = AMMO[env("AMMO", 8.0) as usize].config(1.0);
    c.design.projectile = p;
    c.velocity = env("V", 940.0);
    c.obliquity_deg = env("A", 0.0);
    let dd = c.design.projectile.max_radius() * 2.0;
    let f = (env("NODES", 12.0) * c.dx / dd).clamp(0.004, 6.0);
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
    eprintln!("escala {f:.3}, comprimento {:.1} mm, ø {:.1} mm, peças {}", c.design.projectile.length() * 1e3 / f, dd * 1e3, c.design.n_proj_parts());
    let mut sim = match fem::run::FemSim::new(c) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ERRO na montagem: {e}");
            return;
        }
    };
    eprintln!("{} elementos, passo {:.2e}", sim.hex().len(), sim.dt());
    let t = std::time::Instant::now();
    let mut last = -1.0;
    while !sim.done() {
        sim.advance_for(std::time::Duration::from_millis(200));
        let tm = sim.time() / f * 1e6;
        if tm - last >= env("EVERY", 10.0) || sim.done() {
            last = tm;
            let vm = sim.fem.v.iter().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()).fold(0.0, f64::max);
            let o = sim.outcome();
            eprintln!("t={:6.1} µs  vmax {:10.0}  erodidos {}  energia {:.2}%  passo {:.2e}  v_res {:.0}  ({:.0} s)", tm, vm, sim.fem.n_eroded, o.energy_error_pct, sim.dt(), o.v_residual, t.elapsed().as_secs_f64());
        }
    }
}
