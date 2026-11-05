use pensim_core::*;
fn env(k: &str, d: f64) -> f64 { std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d) }
fn main() {
    let name = std::env::var("AMMO").unwrap_or("M2 AP".into());
    let a = AMMO.iter().find(|a| a.name.contains(&name)).unwrap();
    let nodes = env("NODES", 8.0);
    let s = nodes * 1.5e-3 / (a.caliber_mm * 1e-3);
    let mut c = a.config(s);
    let plate = env("PLATE", a.plate_mm);
    c.design.layers[0].thickness = plate * 1e-3 * s;
    adjust_rha(&mut c.design.layers[0].material, plate * 1e-3);
    let ry = env("RY", c.design.layers[0].material.yield_strength / 1e9);
    c.design.layers[0].material.yield_strength = ry * 1e9;
    c.design.layers[0].material.hardening = env("RH", c.design.layers[0].material.hardening / 1e9) * 1e9;
    c.rate_gain = env("RG", 1.0);
    c.design.projectile.material.fail_stretch = env("PF", c.design.projectile.material.fail_stretch);
    c.design.projectile.material.yield_strength = env("PY", c.design.projectile.material.yield_strength / 1e9) * 1e9;
    c.n_frames = 4;
    let (lo, hi) = (env("VMIN", 500.0), env("VMAX", 1300.0));
    let (pts, v50) = sweep::find_ballistic_limit(&c, lo, hi, 5, 3, |_, _| {});
    let list: Vec<String> = pts.iter().map(|p| format!("{:.0}:{}", p.velocity, if p.outcome.kind == OutcomeKind::Perforation { "P" } else { "-" })).collect();
    println!("{} vs {:.0} mm RHA (σy={:.2} GPa, ganho dinâmico ×{}): V50 ≈ {:?} m/s  [{}]", a.name, plate, ry, c.rate_gain, v50.map(|v| v.round()), list.join(" "));
}
