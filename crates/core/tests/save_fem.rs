//! Arquivo salvo guarda hexaedros e elementos vivos (núcleo de elementos finitos).
use pensim_core::save::{self, SaveData};
use pensim_core::*;

#[test]
fn save_roundtrip_keeps_hex_and_alive() {
    let a = &AMMO[0];
    let mut c = a.config(a.scale);
    c.duration = 10e-6;
    c.n_frames = 3;
    let mut s = fem::run::FemSim::new(c.clone()).unwrap();
    while !s.done() {
        s.advance_for(std::time::Duration::from_millis(200));
    }
    let frames: Vec<_> = s.frames.drain(..).map(std::sync::Arc::new).collect();
    let o = s.outcome();
    let d = SaveData {
        title: "t".into(), velocity: c.velocity, obliquity_deg: 0.0, dx: c.dx, scale: 1.0, design: c.design.clone(),
        n_phys: s.n_physical(), dt: s.dt(), steps: s.steps(), wall: 0.0, t_end: s.t_end(), back_x: s.plate_back(), sym: s.symmetry(),
        outcome: o, parts: s.part_ids(), surface: s.surface(), normals: s.normals(), ref_pos: s.ref_positions(), hex: s.hex().to_vec(), frames: frames.clone(),
    };
    let dir = std::env::temp_dir().join(format!("pensim_save_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("a.psim");
    save::write(&p, &d).unwrap();
    let r = save::read(&p).expect("leitura");
    assert_eq!(r.hex, d.hex);
    assert_eq!(r.frames.len(), frames.len());
    for (a, b) in r.frames.iter().zip(&frames) {
        assert_eq!(a.alive.as_deref(), b.alive.as_deref());
    }
    let _ = std::fs::remove_dir_all(dir);
}
