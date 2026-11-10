//! Packs de munição (`.ppack`): vários projéteis desenhados + dados de cada um, num arquivo só.
//!
//! O pack é só dados (nada de código) e fica fora do repositório do programa: é conteúdo do autor,
//! com a licença dele (docs/NOVO-NUCLEO.md, seção 9). Formato: texto comprimido com zstd.
//!
//! ```text
//! PENSIM-PACK 1
//! N <nome do pack>
//! A <autor>
//! L <licença, \n como "\\n">
//! @AMMO
//! name=… group=… cal=<mm> vel=<m/s> mass=<g> note=…   (uma chave por linha)
//! @DRAWING
//! <texto do .pdes>
//! @END
//! ```
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct PackAmmo {
    pub name: String,
    pub group: String,
    pub caliber_mm: f64,
    pub velocity: f64,
    pub mass_g: f64,
    pub note: String,
    /// texto do desenho (`Drawing::to_text`)
    pub drawing: String,
}

#[derive(Clone, Debug, Default)]
pub struct Pack {
    pub name: String,
    pub author: String,
    pub license: String,
    pub ammo: Vec<PackAmmo>,
    pub path: PathBuf,
}

fn esc(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}
fn unesc(s: &str) -> String {
    let mut o = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n') => o.push('\n'),
                Some(x) => o.push(x),
                None => {}
            }
        } else {
            o.push(c);
        }
    }
    o
}

pub fn to_text(p: &Pack) -> String {
    let mut o = format!("PENSIM-PACK 1\nN {}\nA {}\nL {}\n", esc(&p.name), esc(&p.author), esc(&p.license));
    for a in &p.ammo {
        o += "@AMMO\n";
        o += &format!("name={}\ngroup={}\ncal={}\nvel={}\nmass={}\nnote={}\n", esc(&a.name), esc(&a.group), a.caliber_mm, a.velocity, a.mass_g, esc(&a.note));
        o += "@DRAWING\n";
        o += &a.drawing;
        if !a.drawing.ends_with('\n') {
            o.push('\n');
        }
        o += "@END\n";
    }
    o
}

pub fn from_text(t: &str) -> Option<Pack> {
    let mut lines = t.lines();
    if !lines.next()?.starts_with("PENSIM-PACK") {
        return None;
    }
    let mut p = Pack::default();
    let mut cur: Option<PackAmmo> = None;
    let mut in_draw = false;
    for ln in lines {
        if in_draw {
            if ln == "@END" {
                in_draw = false;
                p.ammo.push(cur.take()?);
            } else if let Some(a) = cur.as_mut() {
                a.drawing.push_str(ln);
                a.drawing.push('\n');
            }
            continue;
        }
        match ln {
            "@AMMO" => cur = Some(PackAmmo::default()),
            "@DRAWING" => in_draw = true,
            _ => {
                if let Some(a) = cur.as_mut() {
                    let (k, v) = ln.split_once('=')?;
                    match k {
                        "name" => a.name = unesc(v),
                        "group" => a.group = unesc(v),
                        "cal" => a.caliber_mm = v.parse().unwrap_or(0.0),
                        "vel" => a.velocity = v.parse().unwrap_or(0.0),
                        "mass" => a.mass_g = v.parse().unwrap_or(0.0),
                        "note" => a.note = unesc(v),
                        _ => {}
                    }
                } else if let Some(r) = ln.strip_prefix("N ") {
                    p.name = unesc(r);
                } else if let Some(r) = ln.strip_prefix("A ") {
                    p.author = unesc(r);
                } else if let Some(r) = ln.strip_prefix("L ") {
                    p.license = unesc(r);
                }
            }
        }
    }
    Some(p)
}

pub fn write(path: &Path, p: &Pack) -> std::io::Result<()> {
    let z = zstd::bulk::compress(to_text(p).as_bytes(), 9).map_err(std::io::Error::other)?;
    std::fs::write(path, z)
}

pub fn read(path: &Path) -> Option<Pack> {
    let z = std::fs::read(path).ok()?;
    let raw = zstd::bulk::decompress(&z, 64 << 20).ok()?;
    let mut p = from_text(std::str::from_utf8(&raw).ok()?)?;
    p.path = path.to_path_buf();
    Some(p)
}

/// Todos os packs válidos da pasta.
pub fn list(dir: &Path) -> Vec<Pack> {
    let mut v: Vec<Pack> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "ppack"))
        .filter_map(|p| read(&p))
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let p = Pack {
            name: "Pack X".into(),
            author: "Fulano".into(),
            license: "Uso pessoal.\nNão redistribuir.".into(),
            ammo: vec![PackAmmo { name: "Bala = 1".into(), group: "G".into(), caliber_mm: 7.62, velocity: 850.0, mass_g: 9.6, note: "a\\b".into(), drawing: "PENSIM-DESENHO 1\nN x\nP 0 0\n".into() }],
            path: PathBuf::new(),
        };
        let q = from_text(&to_text(&p)).unwrap();
        assert_eq!(q.name, p.name);
        assert_eq!(q.license, p.license);
        assert_eq!(q.ammo.len(), 1);
        assert_eq!(q.ammo[0].name, p.ammo[0].name);
        assert_eq!(q.ammo[0].note, p.ammo[0].note);
        assert_eq!(q.ammo[0].drawing, p.ammo[0].drawing);
        assert_eq!(q.ammo[0].velocity, 850.0);
    }
}
