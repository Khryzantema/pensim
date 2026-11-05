//! Threads de trabalho: a simulação nunca roda na thread de UI.
//! Cada passo do solver ainda é paralelizado internamente com rayon.
use pensim_core::sweep::{self, SweepPoint};
use pensim_core::{Config, Frame, Outcome, Sim};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct Shared {
    pub frames: Vec<Arc<Frame>>,
    pub live: Option<Outcome>,
    pub done: bool,
    pub error: Option<String>,
    pub n: usize,
    pub dt: f64,
    pub t_end: f64,
    pub steps: u64,
    pub wall: f64,
    pub back_x: f64,
    pub parts: Arc<Vec<u8>>,
    pub surface: Arc<Vec<bool>>,
    pub ref_pos: Arc<Vec<[f32; 3]>>,
}

pub struct Job {
    pub shared: Arc<Mutex<Shared>>,
    pub paused: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

impl Job {
    pub fn spawn(cfg: Config, ctx: eframe::egui::Context) -> Job {
        let shared = Arc::new(Mutex::new(Shared::default()));
        let paused = Arc::new(AtomicBool::new(false));
        let cancel = Arc::new(AtomicBool::new(false));
        let (s, p, c) = (shared.clone(), paused.clone(), cancel.clone());
        std::thread::spawn(move || {
            let mut sim = match Sim::new(cfg) {
                Ok(s) => s,
                Err(e) => {
                    let mut g = s.lock().unwrap();
                    g.error = Some(e);
                    g.done = true;
                    ctx.request_repaint();
                    return;
                }
            };
            {
                let mut g = s.lock().unwrap();
                g.n = sim.n();
                g.dt = sim.dt;
                g.t_end = sim.t_end();
                g.back_x = sim.plate_back();
                g.parts = Arc::new(sim.part_ids());
                g.surface = Arc::new(sim.surface());
                g.ref_pos = Arc::new(sim.ref_positions());
            }
            let mut wall = 0.0;
            while !c.load(Ordering::SeqCst) && !sim.done() {
                if p.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(20));
                    continue;
                }
                let t = Instant::now();
                sim.advance_for(Duration::from_millis(60));
                wall += t.elapsed().as_secs_f64();
                let new: Vec<Arc<Frame>> = sim.frames.drain(..).map(Arc::new).collect();
                let live = sim.outcome();
                let mut g = s.lock().unwrap();
                g.frames.extend(new);
                g.live = Some(live);
                g.steps = sim.step_count;
                g.wall = wall;
                g.t_end = sim.t_end();
                g.done = sim.done();
                drop(g);
                ctx.request_repaint();
            }
        });
        Job { shared, paused, cancel }
    }
}

#[derive(Default)]
pub struct SweepState {
    pub points: Vec<SweepPoint>,
    pub v50: Option<f64>,
    pub done_runs: usize,
    pub total_runs: usize,
    pub finished: bool,
}

pub struct SweepJob {
    pub state: Arc<Mutex<SweepState>>,
}

impl SweepJob {
    pub fn spawn(cfg: Config, vmin: f64, vmax: f64, n: usize, refine: usize, ctx: eframe::egui::Context) -> SweepJob {
        let state = Arc::new(Mutex::new(SweepState { total_runs: n + refine, ..Default::default() }));
        let st = state.clone();
        std::thread::spawn(move || {
            let st2 = st.clone();
            let ctx2 = ctx.clone();
            let (pts, v50) = sweep::find_ballistic_limit(&cfg, vmin, vmax, n, refine, move |d, t| {
                let mut g = st2.lock().unwrap();
                g.done_runs = d;
                g.total_runs = t;
                ctx2.request_repaint();
            });
            let mut g = st.lock().unwrap();
            g.points = pts;
            g.v50 = v50;
            g.finished = true;
            ctx.request_repaint();
        });
        SweepJob { state }
    }
}
