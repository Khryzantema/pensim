use pensim_core::*;
fn main() {
    for a in AMMO {
        let c = a.config(a.scale);
        let m = Model::build(&c).unwrap();
        let np = m.part.iter().filter(|&&p| p < m.n_proj).count();
        println!("{:48} escala {:.2} dx {:.2}mm nós {:6} (proj {:5}) massa modelo {:.2} g v={}", a.name, a.scale, c.dx*1e3, m.pos.len(), np, a.scaled_mass_g(a.scale), a.velocity);
    }
}
