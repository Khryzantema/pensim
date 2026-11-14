//! Maior velocidade de nó em cada quadro de um .psim (procura o começo de uma instabilidade).
fn main() {
    let p = std::env::args().nth(1).expect("arquivo");
    let d = pensim_core::save::read(std::path::Path::new(&p)).unwrap();
    for (k, fr) in d.frames.iter().enumerate() {
        let f = fr.unpack();
        let (mut vm, mut im) = (0.0f32, 0);
        for (i, &s) in f.speed.iter().enumerate() {
            if s > vm {
                vm = s;
                im = i;
            }
        }
        let alive = f.alive.as_ref().map_or(0, |a| a.iter().map(|b| b.count_ones()).sum::<u32>());
        println!("{k:4} t={:7.2} µs vmax {:12.0} nó {im:6} vivos {alive}", f.time * 1e6, vm);
    }
}
