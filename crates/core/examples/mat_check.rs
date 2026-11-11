//! Imprime como cada material da biblioteca vira material do núcleo novo (A, B, n, dano).
use pensim_core::fem::material::from_library;
fn main() {
    for m in pensim_core::LIBRARY {
        let f = from_library(m);
        println!("{:48} rho {:6.0} A {:6.0} MPa B {:5.0} n {:.2} dano {}", m.name, f.rho, f.jc.a / 1e6, f.jc.b / 1e6, f.jc.n, if f.dmg.is_some() { "JC" } else { "-" });
    }
}
