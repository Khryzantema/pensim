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
    let f = s.frames.last().unwrap();
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
        if a.core.is_some() {
            assert!(m.part.iter().any(|&p| p == 1), "{}: sem nós de núcleo", a.name);
            assert_eq!(m.n_proj, 2);
        }
    }
}

#[test]
fn scaled_projectile_with_core_runs() {
    let a = AMMO.iter().find(|a| a.name.contains("M61")).unwrap();
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
    assert_eq!(full.kind, half.kind);
    assert!((full.v_residual - half.v_residual).abs() < 2.0 + 0.02 * full.v_residual.abs(), "vres {} vs {}", full.v_residual, half.v_residual);
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
