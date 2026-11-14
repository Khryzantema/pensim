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
        if let Some(al) = &f.alive {
            let mut live = vec![false; f.pos.len()];
            for (e, h) in d.hex.iter().enumerate() {
                if al[e / 8] & (1 << (e % 8)) != 0 {
                    for &q in h {
                        live[q as usize] = true;
                    }
                }
            }
            let dmg1 = (0..f.pos.len()).filter(|&i| f.damage[i] >= 0.995).count();
            let loose = (0..f.pos.len()).filter(|&i| !live[i]).count();
            let both = (0..f.pos.len()).filter(|&i| !live[i] && f.damage[i] >= 0.995).count();
            println!("   dano>=0,995: {dmg1}   sem elemento vivo: {loose}   ambos: {both}");
            // fragmento dentro de elemento vivo (teste pelos planos das faces, sem grade: lento mas simples)
            let faces = pensim_core::fem::mesh::FACES;
            let mut inside = 0;
            let mut ex = Vec::new();
            for i in (0..f.pos.len()).filter(|&i| !live[i] && f.damage[i] >= 0.995) {
                let p = f.pos[i].map(|v| v as f64);
                for (e, h) in d.hex.iter().enumerate() {
                    if al[e / 8] & (1 << (e % 8)) == 0 { continue; }
                    let c: [[f64; 3]; 8] = h.map(|q| f.pos[q as usize].map(|v| v as f64));
                    let ec = [0, 1, 2].map(|a| c.iter().map(|q| q[a]).sum::<f64>() / 8.0);
                    let d2 = (0..3).map(|a| (p[a] - ec[a]).powi(2)).sum::<f64>();
                    if d2 > 4e-6 { continue; }
                    let mut ins = true;
                    for fl in faces {
                        let q = fl.map(|l| c[l]);
                        let cen = [0, 1, 2].map(|a| 0.25 * (q[0][a] + q[1][a] + q[2][a] + q[3][a]));
                        let u = [q[2][0] - q[0][0], q[2][1] - q[0][1], q[2][2] - q[0][2]];
                        let v = [q[3][0] - q[1][0], q[3][1] - q[1][1], q[3][2] - q[1][2]];
                        let mut nn = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
                        if (0..3).map(|a| (cen[a] - ec[a]) * nn[a]).sum::<f64>() < 0.0 { nn = nn.map(|x| -x); }
                        if (0..3).map(|a| (p[a] - cen[a]) * nn[a]).sum::<f64>() > 0.0 { ins = false; break; }
                    }
                    if ins { inside += 1; if ex.len() < 5 { ex.push((i, e, d.parts[i], p.map(|v| (v * 1e4).round() / 10.0))); } break; }
                }
            }
            println!("   fragmentos dentro de elemento vivo: {inside}  exemplos (nó, elem, peça, pos mm) {:?}", ex);
        }
    }
}
