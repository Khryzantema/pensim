//! Munição da biblioteca × chapa (padrão da munição) em ângulo: roda no PenSim e escreve os
//! decks do OpenCourant com a mesma malha. AMMO (trecho do nome), A (graus), NODES (no
//! diâmetro), PLATE (mm, real; padrão = da munição), V (m/s; padrão = de boca), OUT, RUN.
use pensim_core::fem::build::build;
use pensim_core::fem::radioss;
use pensim_core::fem::{Fem, FemModel};
use pensim_core::*;

fn env(k: &str, d: f64) -> f64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn main() {
    let key = std::env::var("AMMO").unwrap_or("M61".into());
    let a = AMMO.iter().find(|a| a.name.contains(&key)).expect("munição");
    let nodes = env("NODES", 12.0);
    let s = nodes * 1.5e-3 / (a.caliber_mm * 1e-3);
    let mut c = a.config(s);
    c.dx = 1.5e-3;
    c.obliquity_deg = env("A", 0.0);
    c.velocity = env("V", a.velocity);
    // SPIN em mil rpm reais; modelo em escala s: ω_modelo = ω_real / s
    c.spin = env("SPIN", 0.0) * 1000.0 * std::f64::consts::TAU / 60.0 / s;
    let plate_mm = env("PLATE", a.plate_mm);
    c.design.layers[0].thickness = plate_mm * 1e-3 * s;
    adjust_rha(&mut c.design.layers[0].material, plate_mm * 1e-3);
    let len = c.design.projectile.length();
    let tt = c.design.layers[0].thickness / c.obliquity().cos();
    let dur = 1.5 * (2.0 * len + 3.0 * tt) / c.velocity;
    let st = build(&c).unwrap();
    let mut node_part = vec![u8::MAX; st.mesh.n_nodes()];
    for (e, h) in st.mesh.hex.iter().enumerate() {
        for &k in h {
            node_part[k as usize] = node_part[k as usize].min(st.mesh.part[e]);
        }
    }
    let pnodes: Vec<u32> = (0..st.mesh.n_nodes() as u32).filter(|&i| node_part[i as usize] < st.n_proj).collect();
    let out = std::env::var("OUT").unwrap_or("/tmp/calib".into());
    std::fs::create_dir_all(&out).unwrap();
    let short: String = a.name.split_whitespace().take(2).collect::<Vec<_>>().join("").chars().filter(|ch| ch.is_ascii_alphanumeric()).collect();
    let name = format!("mun_{}_{:.0}mm_{:.0}g_{:.0}ms_n{:.0}_{:.0}krpm", short, plate_mm, c.obliquity_deg, c.velocity, nodes, env("SPIN", 0.0));
    let (sd, ed) = radioss::write(&radioss::DeckInput { name: &name, mesh: &st.mesh, mats: &st.mats, v0: &st.v0, friction: 0.1, t_end: dur, th_dt: dur / 200.0, isolid: 24, th_nodes: &pnodes, dt_del: 0.0 });
    // NODECK=1: o deck já foi escrito (o OpenCourant pode estar lendo ele agora)
    if env("NODECK", 0.0) < 0.5 {
        std::fs::write(format!("{out}/{name}_0000.rad"), sd).unwrap();
        std::fs::write(format!("{out}/{name}_0001.rad"), ed).unwrap();
    }
    println!("NAME {name}");
    println!("NPROJ {}", st.n_proj);
    {
        let mut mass = vec![0.0; st.mesh.n_nodes()];
        for (e, h) in st.mesh.hex.iter().enumerate() {
            let vol = pensim_core::fem::mesh::hex_volume(&st.mesh.corners(h));
            for &k in h {
                mass[k as usize] += st.mats[st.mesh.part[e] as usize].rho * vol / 8.0;
            }
        }
        let txt: Vec<String> = pnodes.iter().map(|&i| format!("{} {:.9e}", i + 1, mass[i as usize])).collect();
        std::fs::write(format!("{out}/{name}.mass"), txt.join("\n")).unwrap();
    }
    println!("deck {out}/{name}_0000.rad ({} nós, {} elementos, {:.0} µs)", st.mesh.n_nodes(), st.mesh.n_elems(), dur * 1e6);
    if env("RUN", 1.0) < 0.5 {
        return;
    }
    let np = st.n_proj;
    let mut md = FemModel::new(st.mesh, st.mats, st.v0);
    md.contact = true;
    md.friction = 0.1;
    let t0 = std::time::Instant::now();
    let mut sim = Fem::new(md);
    {
        // giro inicial em torno do eixo de voo
        let d = [c.obliquity().cos(), 0.0, -c.obliquity().sin()];
        let (mut l, mut iax, mut cg, mut mt, mut vc) = (0.0, 0.0, [0.0; 3], 0.0, [0.0; 3]);
        for i in 0..sim.n_nodes() {
            if node_part[i] < np {
                let mi = sim.node_mass(i);
                for k in 0..3 {
                    cg[k] += mi * sim.x[i][k];
                    vc[k] += mi * sim.v[i][k];
                }
                mt += mi;
            }
        }
        cg = cg.map(|x| x / mt);
        vc = vc.map(|x| x / mt);
        for i in 0..sim.n_nodes() {
            if node_part[i] < np {
                let mi = sim.node_mass(i);
                let r = [sim.x[i][0] - cg[0], sim.x[i][1] - cg[1], sim.x[i][2] - cg[2]];
                let vr = [sim.v[i][0] - vc[0], sim.v[i][1] - vc[1], sim.v[i][2] - vc[2]];
                let cr = [r[1] * vr[2] - r[2] * vr[1], r[2] * vr[0] - r[0] * vr[2], r[0] * vr[1] - r[1] * vr[0]];
                l += mi * (cr[0] * d[0] + cr[1] * d[1] + cr[2] * d[2]);
                let rd = r[0] * d[0] + r[1] * d[1] + r[2] * d[2];
                iax += mi * (r[0] * r[0] + r[1] * r[1] + r[2] * r[2] - rd * rd);
            }
        }
        println!("GIRO inicial ω = {:.0} rad/s (modelo), {:.0} mil rpm reais", l / iax, l / iax * s * 60.0 / std::f64::consts::TAU / 1000.0);
        if env("ONLY_INIT", 0.0) > 0.5 {
            return;
        }
    }
    sim.run_until(dur);
    let (mut p, mut m) = ([0.0; 3], 0.0);
    for i in 0..sim.n_nodes() {
        if node_part[i] < np {
            let mi = sim.node_mass(i);
            for k in 0..3 {
                p[k] += mi * sim.v[i][k];
            }
            m += mi;
        }
    }
    let v = p.map(|x| x / m);
    // momento angular do projétil em torno do eixo de voo (no fim)
    {
        let d = [c.obliquity().cos(), 0.0, -c.obliquity().sin()];
        let (mut cg, mut mt) = ([0.0; 3], 0.0);
        for i in 0..sim.n_nodes() {
            if node_part[i] < np {
                let mi = sim.node_mass(i);
                for k in 0..3 {
                    cg[k] += mi * sim.x[i][k];
                }
                mt += mi;
            }
        }
        cg = cg.map(|x| x / mt);
        let (mut l, mut iax) = (0.0, 0.0);
        for i in 0..sim.n_nodes() {
            if node_part[i] < np {
                let mi = sim.node_mass(i);
                let r = [sim.x[i][0] - cg[0], sim.x[i][1] - cg[1], sim.x[i][2] - cg[2]];
                let vr = [sim.v[i][0] - v[0], sim.v[i][1] - v[1], sim.v[i][2] - v[2]];
                let c = [r[1] * vr[2] - r[2] * vr[1], r[2] * vr[0] - r[0] * vr[2], r[0] * vr[1] - r[1] * vr[0]];
                l += mi * (c[0] * d[0] + c[1] * d[1] + c[2] * d[2]);
                let rd = r[0] * d[0] + r[1] * d[1] + r[2] * d[2];
                iax += mi * (r[0] * r[0] + r[1] * r[1] + r[2] * r[2] - rd * rd);
            }
        }
        println!("GIRO final ω = {:.0} rad/s (modelo), {:.0} mil rpm reais", l / iax, l / iax * s * 60.0 / std::f64::consts::TAU / 1000.0);
    }
    println!("RESULT {} {:.3} {:.3} {:.3} {} {:.4} {:.2}", name, v[0], v[1], v[2], sim.n_eroded, 100.0 * sim.energy.balance(sim.e0) / sim.e0, t0.elapsed().as_secs_f64());
}
