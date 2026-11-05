use pensim_core::*;
fn run(label: &str, mut c: Config) {
    c.n_frames = 100;
    let est = c.estimate_nodes();
    let t = std::time::Instant::now();
    let mut s = Sim::new(c).unwrap();
    s.run_to_end();
    let run = t.elapsed().as_secs_f64();
    let o = s.outcome();
    let nodes = s.n();
    let (raw, packed): (usize, usize) = s.frames.iter().map(|f| { let fr = f.unpack(); (fr.pos.len() * 36, PackedFrame::pack(&fr, true).size_bytes()) }).fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    println!("{label:<44} nós físicos≈{:>6.0}k calc={:>6} passos={:>5} tempo={:>6.1}s ({:>5.2} ms/passo) {:?} | quadros: {} → cru {:>5.1} MB, zstd {:>5.1} MB (×{:.1})",
        est / 1e3, nodes, s.step_count, run, 1e3 * run / s.step_count as f64, o.kind, s.frames.len(), raw as f64 / 1e6, packed as f64 / 1e6, raw as f64 / packed.max(1) as f64);
}
fn main() {
    let base = |v: f64| { let mut c = Preset::Rifle762.config(); c.velocity = v; c };
    let slow = |mut c: Config| { c.symmetry = false; c.sleep_ratio = 0.0; c };
    println!("--- 7,62 mm vs 16 mm RHA, 900 m/s, malha 1,5 mm");
    run("modo lento (sem simetria, sem região ativa)", slow(base(900.0)));
    run("modo atual (simetria + região ativa)", base(900.0));
    println!("--- mesma, malha 1,0 mm");
    let mut f = base(900.0); f.dx = 0.001;
    run("modo lento", slow(f.clone()));
    run("modo atual", f);
    println!("--- alvo grosso: 60 mm de aço, 1200 m/s");
    let thick = |mut c: Config| { c.design.layers[0].thickness = 0.06; c.design.layers[0].width = 0.07; c.design.layers[0].height = 0.07; c };
    run("modo lento", slow(thick(base(1200.0))));
    run("modo atual", thick(base(1200.0)));
    println!("--- 128 mm PzGr 43 × 300 mm de aço (escala ×0,12)");
    let a = AMMO.iter().find(|a| a.name.contains("128 mm")).unwrap();
    run("modo atual", a.config(a.scale));
}
