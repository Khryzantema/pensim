//! Varredura de velocidades (paralela) e estimativa do limite balístico.
use crate::config::Config;
use crate::sim::{Outcome, OutcomeKind, Sim};
use rayon::prelude::*;

#[derive(Clone, Debug)]
pub struct SweepPoint {
    pub velocity: f64,
    pub outcome: Outcome,
}

pub fn run_one(cfg: &Config, velocity: f64) -> SweepPoint {
    let mut c = cfg.clone();
    c.velocity = velocity;
    c.n_frames = 2;
    let mut s = Sim::new(c).expect("configuração inválida");
    s.run_to_end();
    SweepPoint { velocity, outcome: s.outcome() }
}

/// Executa `n` velocidades em paralelo, depois refina por bissecção `refine` vezes.
pub fn find_ballistic_limit(
    cfg: &Config,
    v_min: f64,
    v_max: f64,
    n: usize,
    refine: usize,
    progress: impl Fn(usize, usize) + Sync,
) -> (Vec<SweepPoint>, Option<f64>) {
    let total = n + refine;
    let done = std::sync::atomic::AtomicUsize::new(0);
    let vs: Vec<f64> =
        (0..n).map(|i| v_min + (v_max - v_min) * i as f64 / (n.max(2) - 1) as f64).collect();
    let mut pts: Vec<SweepPoint> = vs
        .par_iter()
        .map(|&v| {
            let r = run_one(cfg, v);
            let d = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            progress(d, total);
            r
        })
        .collect();
    pts.sort_by(|a, b| a.velocity.total_cmp(&b.velocity));
    for k in 0..refine {
        let lo = pts.iter().rev().find(|p| p.outcome.kind != OutcomeKind::Perforation);
        let hi = pts.iter().find(|p| p.outcome.kind == OutcomeKind::Perforation);
        match (lo, hi) {
            (Some(l), Some(h)) if h.velocity > l.velocity => {
                let mid = 0.5 * (l.velocity + h.velocity);
                pts.push(run_one(cfg, mid));
                pts.sort_by(|a, b| a.velocity.total_cmp(&b.velocity));
            }
            _ => {}
        }
        progress(n + k + 1, total);
    }
    // V50 ≈ média entre a maior parada e a menor perfuração (assumindo monotonia)
    let hi = pts.iter().find(|p| p.outcome.kind == OutcomeKind::Perforation);
    let lo = pts.iter().rev().find(|p| p.outcome.kind != OutcomeKind::Perforation);
    let v50 = match (lo, hi) {
        (Some(l), Some(h)) => Some(0.5 * (l.velocity + h.velocity)),
        _ => None,
    };
    (pts, v50)
}
