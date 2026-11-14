//! Caso de calibração esfera × chapa: roda no PenSim e escreve os decks do OpenCourant com a
//! mesma malha. Variáveis: V (m/s), T (mm), A (graus), ND (elementos no diâmetro), MAT (al|rha),
//! OUT (pasta), RUN=0 para só escrever os decks.
use pensim_core::fem::material::{from_library, lib, FemMat};
use pensim_core::fem::mesh::{self, Symmetry};
use pensim_core::fem::radioss;
use pensim_core::fem::{Fem, FemModel};

fn env(k: &str, d: f64) -> f64 {
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
}

fn main() {
    let mm = 1e-3;
    let v = env("V", 1000.0);
    let t = env("T", 5.0) * mm;
    let ang = env("A", 0.0);
    let nd = env("ND", 12.0) as usize;
    let dur = env("DUR", 40.0) * 1e-6;
    let plate_mat: FemMat = match std::env::var("MAT").unwrap_or("al".into()).as_str() {
        "rha" => {
            let mut m = pensim_core::LIBRARY[0].clone();
            pensim_core::adjust_rha(&mut m, t);
            from_library(&m)
        }
        _ => lib::al_6061_t6(),
    };
    let out = std::env::var("OUT").unwrap_or("/tmp/calib".into());
    std::fs::create_dir_all(&out).unwrap();
    let d = 10.0 * mm;
    let r = d / 2.0;
    let dx = t / (t / (d / nd as f64)).round().max(2.0);
    let sym = if ang == 0.0 { Symmetry { y0: true, z0: true } } else { Symmetry { y0: false, z0: true } };
    let cosa = ang.to_radians().cos();
    let mut m = mesh::sphere(r, [-(r + 0.5 * dx) / cosa, 0.0, 0.0], nd, sym, 0);
    let wl = (3.0 * d / dx).round() * dx;
    let mut plate = mesh::block([0.0, -wl, -wl], [t, wl, wl], dx, sym, 1);
    for i in 0..plate.n_nodes() {
        let p = plate.x[i];
        if p[1].abs() > wl - 1e-9 || p[2].abs() > wl - 1e-9 {
            plate.sym[i] |= 7;
        }
    }
    if ang != 0.0 {
        let (s, c) = ang.to_radians().sin_cos();
        for p in &mut plate.x {
            let (x, y) = (p[0], p[1]);
            p[0] = c * x - s * y;
            p[1] = s * x + c * y;
        }
    }
    let ns = m.n_nodes();
    m.append(&plate);
    m.hex = m.hex.iter().map(|h| mesh::orient(&m.x, *h)).collect();
    let n = m.n_nodes();
    let v0: Vec<[f64; 3]> = (0..n).map(|i| if i < ns { [v, 0.0, 0.0] } else { [0.0; 3] }).collect();
    let mut mats = vec![lib::steel_4340(), plate_mat];
    // JC_D3 / JC_D5: troca esses termos do dano nos dois materiais (diagnóstico)
    for m in &mut mats {
        if let Some(d) = m.dmg.as_mut() {
            if let Ok(v) = std::env::var("JC_D3") {
                d.d[2] = v.parse().unwrap();
            }
            if let Ok(v) = std::env::var("JC_D5") {
                d.d[4] = v.parse().unwrap();
            }
        }
    }
    // NOEOS=1: equação de estado linear (K = ρc0²) nos dois, sem Mie-Grüneisen (diagnóstico)
    if env("NOEOS", 0.0) > 0.5 {
        for m in &mut mats {
            let k = m.bulk();
            m.eos = pensim_core::fem::material::Eos::Linear { k };
        }
    }
    // NODMG=1: sem dano Johnson-Cook nos dois solvers (diagnóstico: separa dano de plasticidade)
    if env("NODMG", 0.0) > 0.5 {
        for m in &mut mats {
            m.dmg = None;
        }
    }
    let name = format!("esf_{}_{:.1}mm_{:.0}g_{:.0}ms_nd{}", std::env::var("MAT").unwrap_or("al".into()), t / mm, ang, v, nd);
    // passo inicial do PenSim: o OpenCourant apaga elementos abaixo de 10% dele (mesma regra)
    let dt0 = {
        let mut md = FemModel::new(m.clone(), mats.clone(), v0.clone());
        md.contact = true;
        Fem::new(md).dt
    };
    let (st, en) = radioss::write(&radioss::DeckInput { name: &name, mesh: &m, mats: &mats, v0: &v0, friction: env("FRIC", 0.1), t_end: dur, th_dt: 0.5e-6, isolid: env("ISOLID", 24.0) as i32, th_nodes: &(0..ns as u32).collect::<Vec<_>>(), dt_del: env("DTFRAC", 0.1) * dt0 });
    // NODECK=1: o deck já foi escrito (o OpenCourant pode estar lendo ele agora)
    if env("NODECK", 0.0) < 0.5 {
        std::fs::write(format!("{out}/{name}_0000.rad"), st).unwrap();
        std::fs::write(format!("{out}/{name}_0001.rad"), en).unwrap();
    }
    println!("NAME {name}");
    // massas nodais (concentradas, iguais às do Radioss) para a média do lado do OpenCourant
    {
        let mut mass = vec![0.0; m.n_nodes()];
        for (e, h) in m.hex.iter().enumerate() {
            let vol = mesh::hex_volume(&m.corners(h));
            for &k in h {
                mass[k as usize] += mats[m.part[e] as usize].rho * vol / 8.0;
            }
        }
        let txt: Vec<String> = (0..ns).map(|i| format!("{} {:.9e}", i + 1, mass[i])).collect();
        std::fs::write(format!("{out}/{name}.mass"), txt.join("\n")).unwrap();
    }
    println!("deck {out}/{name}_0000.rad ({} nós, {} elementos)", n, m.n_elems());
    if env("RUN", 1.0) < 0.5 {
        return;
    }
    let mut md = FemModel::new(m, mats, v0);
    md.contact = true;
    md.friction = env("FRIC", 0.1);
    let t0 = std::time::Instant::now();
    let mut sim = Fem::new(md);
    if env("PROBE", 0.0) > 0.5 {
        // a cada 4 µs: 1% mais tracionado da pressão por peça e fração com triaxialidade > 1
        let mut t = 0.0;
        while t < dur - 1e-12 {
            t += env("PROBE_DT", 4.0) * 1e-6;
            sim.run_until(t.min(dur));
            for part in 0..2u8 {
                let mut ps: Vec<f64> = (0..sim.n_elems()).filter(|&e| sim.mesh.part[e] == part && sim.is_alive(e)).map(|e| sim.pressure(e)).collect();
                ps.sort_by(|a, b| a.total_cmp(b));
                let k = (ps.len() / 100).max(1);
                let p1 = ps[..k].iter().sum::<f64>() / k as f64;
                let p99 = ps[ps.len() - k..].iter().sum::<f64>() / k as f64;
                let tri = (0..sim.n_elems()).filter(|&e| sim.mesh.part[e] == part && sim.is_alive(e) && -sim.pressure(e) > sim.von_mises(e)).count();
                let dm: Vec<f64> = (0..sim.n_elems()).filter(|&e| sim.mesh.part[e] == part).map(|e| if sim.is_alive(e) { sim.damage(e).min(1.0) } else { 1.0 }).collect();
                let n5 = dm.iter().filter(|&&d| d >= 0.5).count();
                let n1 = dm.iter().filter(|&&d| d >= 0.999).count();
                let n2 = dm.iter().filter(|&&d| d >= 0.2).count();
                println!("PROBE t={:.0}us peca {} p1%={:.2} p99%={:.2} GPa  triax>1: {}  dano>=0.2: {} >=0.5: {} rompidos: {}", t * 1e6, part, p1 / 1e9, p99 / 1e9, tri, n2, n5, n1);
            }
        }
    } else {
        sim.run_until(dur);
    }
    let vp = sim.part_velocity(0);
    println!("EROSAO [dano, fragil, distorcao, volume] = {:?}", sim.erosion_reasons());
    {
        let mut per = [0usize; 2];
        for (e, &a) in sim.alive.iter().enumerate() {
            if !a {
                per[sim.mesh.part[e].min(1) as usize] += 1;
            }
        }
        println!("APAGADOS esfera {} chapa {}", per[0], per[1]);
        // deformação plástica máxima e média dos elementos da chapa sob a esfera (vivos ou não)
        let ne = sim.n_elems();
        let mut eps: Vec<f64> = (0..ne).filter(|&e| sim.mesh.part[e] == 1).map(|e| sim.eps_p(e)).collect();
        eps.sort_by(|a, b| b.total_cmp(a));
        let top: f64 = eps.iter().take(300).sum::<f64>() / 300.0;
        println!("EPS_P chapa: max {:.3}, média dos 300 maiores {:.3}", eps[0], top);
    }
    println!("RESULT {} {:.3} {:.3} {:.3} {} {:.4} {:.2}", name, vp[0], vp[1], vp[2], sim.n_eroded, 100.0 * sim.energy.balance(sim.e0) / sim.e0, t0.elapsed().as_secs_f64());
    println!("PenSim: v_cg esfera = ({:.1}, {:.1}, {:.1}) m/s, |v| = {:.1}, erodidos {}, energia {:.2}%, {:.1} s", vp[0], vp[1], vp[2], (vp[0] * vp[0] + vp[1] * vp[1] + vp[2] * vp[2]).sqrt(), sim.n_eroded, 100.0 * sim.energy.balance(sim.e0) / sim.e0, t0.elapsed().as_secs_f64());
}
