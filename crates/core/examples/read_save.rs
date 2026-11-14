//! Lê um .psim e mostra o resultado e as maiores velocidades do último quadro.
fn main() {
    let p = std::env::args().nth(1).expect("arquivo");
    let d = pensim_core::save::read(std::path::Path::new(&p)).unwrap();
    println!("{:?}", d.outcome);
    let n = d.frames.len();
    for &k in &[n / 4, n / 2, 3 * n / 4, n - 1] {
        let f = d.frames[k].unpack();
        let mut sp: Vec<(f32, usize)> = f.speed.iter().cloned().zip(0..).collect();
        sp.sort_by(|a, b| b.0.total_cmp(&a.0));
        println!("quadro {k} t={:.1} µs: maiores velocidades {:?}", f.time * 1e6, &sp[..5]);
    }
}
