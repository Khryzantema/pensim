use pensim_core::*;

fn cfg(v: f64) -> Config {
    let mut c = Preset::Rifle762.config();
    c.dx = 0.002;
    for l in &mut c.design.layers {
        l.width = 0.04;
        l.height = 0.04;
        l.thickness = 0.014;
    }
    c.velocity = v;
    c.n_frames = 10;
    c
}

fn run(v: f64) -> Outcome {
    let mut s = Sim::new(cfg(v)).unwrap();
    s.run_to_end();
    s.outcome()
}

#[test]
fn energy_is_conserved() {
    for v in [400.0, 1000.0] {
        let o = run(v);
        assert!(o.energy_error_pct.abs() < 3.0, "erro {}% em v={v}", o.energy_error_pct);
    }
}

#[test]
fn low_stops_high_perforates() {
    assert_ne!(run(250.0).kind, OutcomeKind::Perforation);
    let hi = run(1500.0);
    assert_eq!(hi.kind, OutcomeKind::Perforation);
    assert!(hi.v_residual > 100.0);
}

#[test]
fn invalid_designs_are_reported() {
    let mut c = cfg(500.0);
    c.design.layers.clear();
    assert!(Sim::new(c).is_err());
    let mut c = cfg(500.0);
    c.design.projectile.profile.truncate(1);
    assert!(Sim::new(c).is_err());
}

#[test]
fn layers_and_oblique_build_and_record_frames() {
    let mut c = Preset::Laminate.config();
    c.dx = 0.002;
    c.obliquity_deg = 30.0;
    c.n_frames = 8;
    for l in &mut c.design.layers {
        l.width = 0.04;
        l.height = 0.04;
    }
    let mut s = Sim::new(c).unwrap();
    let ids = s.part_ids();
    assert!(ids.contains(&0) && ids.contains(&1) && ids.contains(&2));
    s.run_to_end();
    let f = s.frames.last().unwrap().unpack();
    assert!(f.von_mises.iter().all(|v| v.is_finite()));
    assert!(f.pos.iter().all(|p| p.iter().all(|v| v.is_finite())));
}

#[test]
fn sweep_brackets_limit() {
    let (pts, v50) = sweep::find_ballistic_limit(&cfg(0.0), 300.0, 1500.0, 3, 1, |_, _| {});
    assert!(pts.len() >= 4);
    let v = v50.expect("V50");
    assert!(v > 300.0 && v < 1500.0);
}

#[test]
fn very_high_speed_stays_stable() {
    let o = run(2500.0);
    assert!(o.v_residual.is_finite());
    assert!(o.energy_error_pct.abs() < 5.0, "erro {}%", o.energy_error_pct);
}

#[test]
fn stops_shortly_after_projectile_exits() {
    let c = cfg(1500.0);
    let full = c.t_end();
    let mut s = Sim::new(c).unwrap();
    s.run_to_end();
    assert_eq!(s.outcome().kind, OutcomeKind::Perforation);
    assert!(s.time < 0.85 * full, "parou em {:.1} µs de {:.1} µs", s.time * 1e6, full * 1e6);
    assert!(s.t_end() + 3.0 * s.dt >= s.time);
}

#[test]
fn ammo_library_builds_valid_models() {
    for a in AMMO {
        let cfg = a.config(a.scale);
        let m = Model::build(&cfg).unwrap_or_else(|e| panic!("{}: {e}", a.name));
        assert!(m.pos.len() > 200 && m.pos.len() < 150_000, "{}: {} nós", a.name, m.pos.len());
        assert!(m.part.iter().any(|&p| p == 0), "{}", a.name);
        // todas as peças entram na contagem de peças do projétil
        assert_eq!(m.n_proj as usize, cfg.design.n_proj_parts(), "{}", a.name);
        assert!(m.part.iter().all(|&p| (p as usize) < cfg.design.materials().len()), "{}", a.name);
    }
}

#[test]
fn scaled_projectile_with_core_runs() {
    let a = AMMO.iter().find(|a| a.name.contains("M61 (AP)")).unwrap();
    let mut c = a.config(a.scale);
    c.n_frames = 6;
    let mut s = Sim::new(c).unwrap();
    s.run_to_end();
    let o = s.outcome();
    assert!(o.energy_error_pct.abs() < 5.0, "erro {}%", o.energy_error_pct);
}

/// O modelo reduzido por simetria deve reproduzir o modelo completo.
fn sym_vs_full(angle: f64) {
    let mk = |sym: bool| {
        let mut c = cfg(1000.0);
        c.obliquity_deg = angle;
        c.symmetry = sym;
        c.noise = 0.0;
        c.jitter = 0.0;
        c.n_frames = 8;
        let mut s = Sim::new(c).unwrap();
        s.run_to_end();
        let o = s.outcome();
        let ep = s.frames.last().unwrap().e_plastic;
        (o, ep, s.n(), s.n_physical())
    };
    let (full, ef, nf, _) = mk(false);
    let (half, eh, nh, nphys) = mk(true);
    assert!(nh < nf, "simetria deve reduzir os nós ({nh} vs {nf})");
    assert_eq!(nphys, nf, "nós físicos devem bater com o modelo completo");
    // perfurar ou não deve concordar (parcial × rejeitado se confundem no limiar)
    assert_eq!(full.kind == OutcomeKind::Perforation, half.kind == OutcomeKind::Perforation);
    // projétil quase parado → resíduo caótico; compara em relação à velocidade de impacto (1000 m/s)
    assert!((full.v_residual - half.v_residual).abs() < 0.05 * 1000.0, "vres {} vs {}", full.v_residual, half.v_residual);
    assert!((ef - eh).abs() / ef.max(1.0) < 0.01, "energia plástica {ef} vs {eh}");
    assert!(half.energy_error_pct.abs() < 3.0);
}

#[test]
fn symmetry_matches_full_model_normal_impact() {
    sym_vs_full(0.0);
}

#[test]
fn symmetry_matches_full_model_oblique_impact() {
    sym_vs_full(25.0);
}

#[test]
fn packed_frames_roundtrip_both_modes() {
    let mut s = Sim::new(cfg(900.0)).unwrap();
    s.capture();
    let f0 = s.frames[0].unpack();
    for compress in [false, true] {
        let pf = pensim_core::PackedFrame::pack(&f0, compress);
        assert_eq!(pf.is_compressed(), compress);
        let g = pf.unpack();
        assert_eq!(g.pos.len(), f0.pos.len());
        let dpos = f0.pos.iter().zip(&g.pos).map(|(a, b)| (a[0] - b[0]).abs().max((a[1] - b[1]).abs()).max((a[2] - b[2]).abs())).fold(0.0f32, f32::max);
        assert!(dpos < 2e-6, "erro de posição {dpos}");
        assert!(pf.size_bytes() <= f0.pos.len() * 36);
    }
}

#[test]
fn save_and_load_roundtrip() {
    let mut c = cfg(900.0);
    c.n_frames = 6;
    let mut s = Sim::new(c.clone()).unwrap();
    s.run_to_end();
    let frames: Vec<std::sync::Arc<PackedFrame>> = s.frames.drain(..).map(std::sync::Arc::new).collect();
    let data = pensim_core::save::SaveData {
        title: "teste".into(), velocity: c.velocity, obliquity_deg: c.obliquity_deg, dx: c.dx, scale: 1.0, design: c.design.clone(),
        n_phys: s.n_physical(), dt: s.dt, steps: s.step_count, wall: 1.0, t_end: s.t_end(), back_x: s.plate_back(), sym: s.symmetry(),
        outcome: s.outcome(), parts: s.part_ids(), surface: s.surface(), normals: s.normals(), ref_pos: s.ref_positions(), frames: frames.clone(),
    };
    let path = std::env::temp_dir().join("pensim_test.psim");
    let size = pensim_core::save::write(&path, &data).unwrap();
    let info = pensim_core::save::info(&path).unwrap();
    assert_eq!(info.title, "teste");
    assert_eq!(info.frames, frames.len());
    assert_eq!(info.size_bytes, size);
    let back = pensim_core::save::read(&path).unwrap();
    assert_eq!(back.frames.len(), frames.len());
    assert_eq!(back.parts, data.parts);
    assert_eq!(back.design, data.design);
    let (a, b) = (frames[2].unpack(), back.frames[2].unpack());
    assert_eq!(a.pos.len(), b.pos.len());
    assert!((a.e_plastic - b.e_plastic).abs() < 1e-9);
    let _ = std::fs::remove_file(&path);
}

/// Projétil desenhado (contornos) contra um objeto desenhado do alvo, sem chapas.
#[test]
fn drawn_projectile_and_target_object() {
    let mut c = Preset::Rifle762.config();
    c.dx = 0.0015;
    c.n_frames = 4;
    c.velocity = 800.0;
    let mm = 1e-3;
    // bala: camisa de cobre (contorno externo) com núcleo de chumbo (interno)
    let outer = vec![[0.0, 0.0], [10.0 * mm, 4.0 * mm], [28.0 * mm, 4.0 * mm], [28.0 * mm, 0.0]];
    let inner = vec![[4.0 * mm, 0.0], [10.0 * mm, 2.8 * mm], [26.0 * mm, 2.8 * mm], [26.0 * mm, 0.0]];
    let reg = |poly: Vec<[f64; 2]>, fill: Fill| Region { name: "p".into(), outer: vec![], inner: vec![], fill, poly };
    c.design.projectile = Projectile {
        profile: vec![[0.0, 0.0], [10.0 * mm, 4.0 * mm], [28.0 * mm, 4.0 * mm]],
        material: LIBRARY[5].clone(),
        core: None,
        regions: vec![reg(outer, Fill::Body), reg(inner, Fill::Mat(LIBRARY[6].clone()))],
    };
    // alvo: um bloco desenhado de aço (12 × 40 mm na vista lateral, 40 mm de largura)
    c.design.layers.clear();
    c.design.objects.push(TargetObject {
        name: "bloco".into(),
        parts: vec![(vec![[0.0, -20.0 * mm], [12.0 * mm, -20.0 * mm], [12.0 * mm, 20.0 * mm], [0.0, 20.0 * mm]], Fill::Mat(LIBRARY[0].clone()))],
        width: 0.04,
    });
    let m = Model::build(&c).unwrap();
    assert_eq!(m.n_proj as usize, 2);
    assert!(m.part.iter().any(|&p| p == 1), "sem núcleo");
    assert!(m.part.iter().any(|&p| p == 2), "sem alvo");
    let mut s = Sim::new(c).unwrap();
    s.run_to_end();
    let o = s.outcome();
    assert!(o.energy_error_pct.abs() < 5.0, "erro {}%", o.energy_error_pct);
}
