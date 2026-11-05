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
