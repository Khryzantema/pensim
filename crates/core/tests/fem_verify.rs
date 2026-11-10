//! Verificação do núcleo de elementos finitos (docs/NOVO-NUCLEO.md, seção 6.1).
//! Nenhum parâmetro é ajustado aqui: os critérios vêm de soluções analíticas e de conservação.
use pensim_core::fem::material::{lib, FemMat};
use pensim_core::fem::mesh::{self, Symmetry};
use pensim_core::fem::{Fem, FemModel, Prescribed, Wall};

const MM: f64 = 1e-3;

fn steel_elastic() -> FemMat {
    FemMat::elastic("aço elástico", 7800.0, 200e9, 0.3)
}

fn model(m: mesh::HexMesh, mats: Vec<FemMat>, v0: Vec<[f64; 3]>) -> FemModel {
    FemModel { mesh: m, mats, v0, prescribed: vec![], walls: vec![] }
}

/// Instante em que a componente x da velocidade do nó passa de `level` (interpolado).
fn arrival(sim: &mut Fem, node: usize, level: f64, t_max: f64) -> f64 {
    let mut prev = (sim.time, sim.v[node][0]);
    while sim.time < t_max {
        sim.step();
        let cur = (sim.time, sim.v[node][0]);
        if cur.1 >= level {
            let f = (level - prev.1) / (cur.1 - prev.1);
            return prev.0 + f * (cur.0 - prev.0);
        }
        prev = cur;
    }
    panic!("a onda não chegou");
}

fn nodes_at_x(m: &mesh::HexMesh, x: f64) -> Vec<u32> {
    (0..m.n_nodes()).filter(|&i| (m.x[i][0] - x).abs() < 1e-9).map(|i| i as u32).collect()
}

#[test]
fn wave_speed_uniaxial_strain() {
    // barra confinada lateralmente: onda longitudinal c = √((K + 4G/3)/ρ)
    let dx = 1.0 * MM;
    let len = 200.0 * MM;
    let mut m = mesh::block([0.0, 0.0, 0.0], [len, dx, dx], dx, Symmetry::default(), 0);
    for b in &mut m.sym {
        *b |= 2 | 4;
    }
    let mat = steel_elastic();
    let c = ((mat.bulk() + 4.0 / 3.0 * mat.g) / mat.rho).sqrt();
    let rise = 20.0 * dx / c;
    let n = m.n_nodes();
    let mut md = model(m.clone(), vec![mat], vec![[0.0; 3]; n]);
    md.prescribed.push(Prescribed { nodes: nodes_at_x(&m, 0.0), axis: 0, amp: 1.0, rise });
    let mut sim = Fem::new(md);
    let probe = nodes_at_x(&m, 100.0 * MM)[0] as usize;
    let t = arrival(&mut sim, probe, 0.5, 1e-3);
    let c_num = 100.0 * MM / (t - 0.5 * rise);
    let err = (c_num / c - 1.0).abs();
    eprintln!("onda confinada: c = {c_num:.1} m/s, analítico {c:.1} m/s, erro {:.2}%", err * 100.0);
    assert!(err < 0.02);
}

#[test]
fn wave_speed_bar() {
    // barra fina livre: c = √(E/ρ) para comprimento de onda ≫ largura
    let dx = 1.0 * MM;
    let w = 3.0 * MM;
    let len = 300.0 * MM;
    let m = mesh::block([0.0, 0.0, 0.0], [len, w, w], dx, Symmetry::default(), 0);
    let e = 200e9;
    let mat = steel_elastic();
    let c = (e / mat.rho).sqrt();
    let rise = 40.0 * w / c;
    let n = m.n_nodes();
    let mut md = model(m.clone(), vec![mat], vec![[0.0; 3]; n]);
    md.prescribed.push(Prescribed { nodes: nodes_at_x(&m, 0.0), axis: 0, amp: 1.0, rise });
    let mut sim = Fem::new(md);
    let probe = nodes_at_x(&m, 150.0 * MM)[0] as usize;
    let t = arrival(&mut sim, probe, 0.5, 1e-3);
    let c_num = 150.0 * MM / (t - 0.5 * rise);
    let err = (c_num / c - 1.0).abs();
    eprintln!("barra livre: c = {c_num:.1} m/s, analítico {c:.1} m/s, erro {:.2}%", err * 100.0);
    assert!(err < 0.02);
}

/// Cubo 1/8 (simetria em x, y, z = 0) puxado na face +x devagar (quase estático).
fn pull_cube(mat: FemMat, strain: f64) -> (f64, f64, f64, Fem) {
    let dx = 1.0 * MM;
    let l = 4.0 * MM;
    let mut m = mesh::block([0.0, 0.0, 0.0], [l, l, l], dx, Symmetry { y0: true, z0: true }, 0);
    for i in 0..m.n_nodes() {
        if m.x[i][0].abs() < 1e-12 {
            m.sym[i] |= 1;
        }
    }
    let c = ((mat.bulk() + 4.0 / 3.0 * mat.g) / mat.rho).sqrt();
    let t_load = 300.0 * l / c;
    let v = strain * l / t_load;
    let n = m.n_nodes();
    let face = nodes_at_x(&m, l);
    let mut md = model(m.clone(), vec![mat], vec![[0.0; 3]; n]);
    md.prescribed.push(Prescribed { nodes: face.clone(), axis: 0, amp: v, rise: 0.1 * t_load });
    let mut sim = Fem::new(md);
    // a rampa perde 0,05·t_load de deslocamento: estende o tempo para chegar na deformação
    sim.run_until(1.05 * t_load);
    let ne = sim.n_elems();
    let sig = sim.mean_stress(0..ne);
    let xf = sim.x[face[0] as usize][0];
    let eps_eng = (xf - l) / l;
    (sig[0], sig[1], eps_eng, sim)
}

#[test]
fn uniaxial_stress_elastic() {
    let (sxx, syy, eps, _) = pull_cube(steel_elastic(), 1e-3);
    let expect = 200e9 * eps;
    let err = (sxx / expect - 1.0).abs();
    eprintln!("tração elástica: σxx = {:.2} MPa, Eε = {:.2} MPa, erro {:.2}%, σyy/σxx = {:.2e}", sxx / 1e6, expect / 1e6, err * 100.0, syy / sxx);
    assert!(err < 0.01);
    assert!((syy / sxx).abs() < 0.01);
}

#[test]
fn uniaxial_stress_perfect_plastic() {
    let sy = 500e6;
    let (sxx, syy, eps, _) = pull_cube(FemMat::perfectly_plastic("pp", 7800.0, 200e9, 0.3, sy), 0.02);
    let err = (sxx / sy - 1.0).abs();
    eprintln!("tração plástica: ε = {:.3}, σxx = {:.1} MPa (σy = {:.0}), erro {:.2}%, σyy/σxx = {:.2e}", eps, sxx / 1e6, sy / 1e6, err * 100.0, syy / sxx);
    assert!(err < 0.01);
    assert!((syy / sxx).abs() < 0.01);
}

#[test]
fn energy_conserved_free_body() {
    // cubo de cobre (Johnson-Cook + Mie-Grüneisen) com campo de velocidade de compressão +
    // cisalhamento: escoa, aquece e volta; nada de fora faz trabalho
    let dx = 1.0 * MM;
    let l = 10.0 * MM;
    let m = mesh::block([-l / 2.0, -l / 2.0, -l / 2.0], [l / 2.0, l / 2.0, l / 2.0], dx, Symmetry::default(), 0);
    let pi = std::f64::consts::PI;
    let (a, b) = (200.0 / (l / 2.0), 80.0);
    let v0: Vec<[f64; 3]> = m.x.iter().map(|p| [-a * p[0] + b * (pi * p[2] / l).sin(), -a * p[1], -a * p[2] + b * (pi * p[0] / l).sin()]).collect();
    let mut sim = Fem::new(model(m, vec![lib::copper_ofhc()], v0));
    let e0 = sim.e0;
    sim.run_until(30e-6);
    let en = sim.energy;
    let bal = en.balance(e0) / e0;
    let ne = sim.n_elems();
    let ep_max = (0..ne).map(|e| sim.eps_p(e)).fold(0.0, f64::max);
    eprintln!(
        "corpo livre: KE0 = {:.2} J, KE = {:.2}, IE = {:.2}, HG = {:.3} ({:.1}% de IE), erro {:.3}%, εp máx {:.3}, passos {}",
        e0, en.kinetic, en.internal, en.hourglass, 100.0 * en.hourglass / en.internal, 100.0 * bal, ep_max, sim.steps
    );
    assert!(ep_max > 0.02, "o teste tem de plastificar");
    assert!(bal.abs() < 0.02);
    assert!(en.hourglass < 0.1 * en.internal);
}

/// Cilindro de cobre contra parede rígida (geometria do ensaio de Taylor). Devolve
/// (comprimento final, raio máximo, erro de energia, hourglass/interna).
fn taylor(sym: Symmetry, nd: f64) -> (f64, f64, f64, f64, usize) {
    let d = 7.6 * MM;
    let len = 25.4 * MM;
    let dx = d / nd;
    let m = mesh::cylinder_x(d / 2.0, 0.0, len, dx, sym, 0);
    assert!(m.min_jacobian_ratio(dx) > 0.2, "malha ruim: {}", m.min_jacobian_ratio(dx));
    let n = m.n_nodes();
    let ne = m.n_elems();
    let mut md = model(m, vec![lib::copper_ofhc()], vec![[-190.0, 0.0, 0.0]; n]);
    md.walls.push(Wall { axis: 0, pos: 0.0, side: 1.0 });
    let mut sim = Fem::new(md);
    let e0 = sim.e0;
    sim.run_until(80e-6);
    let lf = sim.x.iter().map(|p| p[0]).fold(f64::MIN, f64::max) - sim.x.iter().map(|p| p[0]).fold(f64::MAX, f64::min);
    let rmax = sim.x.iter().map(|p| (p[1] * p[1] + p[2] * p[2]).sqrt()).fold(0.0, f64::max);
    let en = sim.energy;
    (lf, rmax, en.balance(e0) / e0, en.hourglass / en.internal, ne)
}

#[test]
fn rigid_wall_symmetry_and_energy() {
    let nd = 10.0;
    let full = taylor(Symmetry::default(), nd);
    let half = taylor(Symmetry { y0: true, z0: false }, nd);
    let quarter = taylor(Symmetry { y0: true, z0: true }, nd);
    for (nome, r) in [("inteiro", full), ("1/2", half), ("1/4", quarter)] {
        eprintln!(
            "Taylor {nome}: {} elementos, L = {:.2} mm, raio máx = {:.2} mm, erro de energia {:.2}%, HG/IE {:.1}%",
            r.4, r.0 / MM, r.1 / MM, r.2 * 100.0, r.3 * 100.0
        );
        assert!(r.2.abs() < 0.02, "{nome}: energia");
        assert!(r.3 < 0.1, "{nome}: hourglass");
    }
    for r in [half, quarter] {
        assert!((r.0 / full.0 - 1.0).abs() < 0.01, "comprimento final difere");
        assert!((r.1 / full.1 - 1.0).abs() < 0.01, "raio final difere");
    }
}

#[test]
fn sphere_mesh_quality() {
    let r = 5.0 * MM;
    for nd in [8.0, 12.0, 16.0, 24.0] {
        let dx = 2.0 * r / nd;
        let m = mesh::sphere(r, [0.0; 3], dx, Symmetry { y0: true, z0: true }, 0);
        let v_exact = 4.0 / 3.0 * std::f64::consts::PI * r.powi(3) / 4.0;
        let ev = m.volume() / v_exact - 1.0;
        let jr = m.min_jacobian_ratio(dx);
        eprintln!("esfera {nd} no diâmetro: {} elementos, erro de volume {:.2}%, jacobiano mín {:.2}", m.n_elems(), ev * 100.0, jr);
        assert!(ev.abs() < 0.03);
        assert!(jr > 0.2);
    }
}
