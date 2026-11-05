//! Salvar/abrir simulações prontas (arquivo `.psim`): projeto, resultados e quadros comprimidos.
use crate::config::{Core, Design, Layer, Projectile};
use crate::material::{Material, LIBRARY};
use crate::sim::{Outcome, OutcomeKind};
use crate::store::PackedFrame;
use std::sync::Arc;

const MAGIC: &[u8; 4] = b"PSIM";
const VERSION: u32 = 2;

pub struct SaveData {
    pub title: String,
    pub velocity: f64,
    pub obliquity_deg: f64,
    pub dx: f64,
    /// escala do modelo (medidas reais = modelo ÷ escala)
    pub scale: f64,
    pub design: Design,
    pub n_phys: usize,
    pub dt: f64,
    pub steps: u64,
    pub wall: f64,
    pub t_end: f64,
    pub back_x: f64,
    pub sym: u8,
    pub outcome: Outcome,
    pub parts: Vec<u8>,
    pub surface: Vec<bool>,
    pub normals: Vec<[f32; 3]>,
    pub ref_pos: Vec<[f32; 3]>,
    pub frames: Vec<Arc<PackedFrame>>,
}

/// Cabeçalho para listar arquivos sem carregá-los.
pub struct SaveInfo {
    pub title: String,
    pub frames: usize,
    pub size_bytes: u64,
}

struct W(Vec<u8>);
impl W {
    fn u8(&mut self, v: u8) { self.0.push(v) }
    fn u32(&mut self, v: u32) { self.0.extend_from_slice(&v.to_le_bytes()) }
    fn u64(&mut self, v: u64) { self.0.extend_from_slice(&v.to_le_bytes()) }
    fn f64(&mut self, v: f64) { self.0.extend_from_slice(&v.to_le_bytes()) }
    fn str(&mut self, s: &str) { self.u32(s.len() as u32); self.0.extend_from_slice(s.as_bytes()) }
    fn profile(&mut self, p: &[[f64; 2]]) { self.u32(p.len() as u32); for q in p { self.f64(q[0]); self.f64(q[1]); } }
}

struct R<'a>(&'a [u8]);
impl<'a> R<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n { return None; }
        let (a, b) = self.0.split_at(n);
        self.0 = b;
        Some(a)
    }
    fn u8(&mut self) -> Option<u8> { Some(self.take(1)?[0]) }
    fn u32(&mut self) -> Option<u32> { Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?)) }
    fn u64(&mut self) -> Option<u64> { Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?)) }
    fn f64(&mut self) -> Option<f64> { Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?)) }
    fn str(&mut self) -> Option<String> { let n = self.u32()? as usize; Some(String::from_utf8_lossy(self.take(n)?).into_owned()) }
    fn profile(&mut self) -> Option<Vec<[f64; 2]>> {
        let n = self.u32()? as usize;
        (0..n).map(|_| Some([self.f64()?, self.f64()?])).collect()
    }
}

fn material(name: &str) -> Material {
    LIBRARY.iter().find(|m| m.name == name).cloned().unwrap_or_else(|| LIBRARY[0].clone())
}

pub fn write(path: &std::path::Path, d: &SaveData) -> std::io::Result<u64> {
    let mut w = W(Vec::new());
    w.0.extend_from_slice(MAGIC);
    w.u32(VERSION);
    w.str(&d.title);
    w.u32(d.frames.len() as u32);
    for v in [d.velocity, d.obliquity_deg, d.dx, d.dt, d.wall, d.t_end, d.back_x, d.scale] {
        w.f64(v);
    }
    w.u64(d.n_phys as u64);
    w.u64(d.steps);
    w.u8(d.sym);
    // projeto
    let p = &d.design.projectile;
    w.str(p.material.name);
    w.profile(&p.profile);
    match &p.core {
        Some(c) => { w.u8(1); w.str(c.material.name); w.profile(&c.profile); }
        None => w.u8(0),
    }
    w.u32(d.design.layers.len() as u32);
    for l in &d.design.layers {
        for v in [l.thickness, l.gap_before, l.width, l.height] { w.f64(v); }
        w.u8(l.round as u8);
        w.str(l.material.name);
    }
    // resultado
    let o = &d.outcome;
    w.u8(match o.kind { OutcomeKind::Perforation => 0, OutcomeKind::PartialPenetration => 1, OutcomeKind::Rejected => 2 });
    for v in [o.v_residual, o.penetration, o.energy_absorbed_pct, o.target_ejected_pct, o.proj_behind_pct, o.energy_error_pct] { w.f64(v); }
    // geometria de referência (comprimida)
    let mut g: Vec<u8> = Vec::new();
    let n = d.parts.len();
    g.extend_from_slice(&(n as u64).to_le_bytes());
    g.extend_from_slice(&d.parts);
    g.extend(d.surface.iter().map(|&b| b as u8));
    for q in d.normals.iter().chain(d.ref_pos.iter()) {
        for v in q { g.extend_from_slice(&v.to_le_bytes()); }
    }
    let gz = zstd::bulk::compress(&g, 3).unwrap_or(g);
    w.u64(gz.len() as u64);
    w.0.extend_from_slice(&gz);
    for f in &d.frames {
        f.write_to(&mut w.0);
    }
    std::fs::write(path, &w.0)?;
    Ok(w.0.len() as u64)
}

/// Lê só o cabeçalho (título e nº de quadros).
pub fn info(path: &std::path::Path) -> Option<SaveInfo> {
    use std::io::Read;
    let size = std::fs::metadata(path).ok()?.len();
    let mut buf = vec![0u8; 512];
    let n = std::fs::File::open(path).ok()?.read(&mut buf).ok()?;
    let mut r = R(&buf[..n]);
    if r.take(4)? != MAGIC || r.u32()? != VERSION {
        return None;
    }
    let title = r.str()?;
    let frames = r.u32()? as usize;
    Some(SaveInfo { title, frames, size_bytes: size })
}

pub fn read(path: &std::path::Path) -> Option<SaveData> {
    let bytes = std::fs::read(path).ok()?;
    let mut r = R(&bytes);
    if r.take(4)? != MAGIC || r.u32()? != VERSION {
        return None;
    }
    let title = r.str()?;
    let nframes = r.u32()? as usize;
    let (velocity, obliquity_deg, dx, dt, wall, t_end, back_x, scale) = (r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?);
    let n_phys = r.u64()? as usize;
    let steps = r.u64()?;
    let sym = r.u8()?;
    let pm = material(&r.str()?);
    let profile = r.profile()?;
    let core = if r.u8()? == 1 {
        let m = material(&r.str()?);
        Some(Core { profile: r.profile()?, material: m })
    } else {
        None
    };
    let nl = r.u32()? as usize;
    let mut layers = Vec::new();
    for _ in 0..nl {
        let (thickness, gap_before, width, height) = (r.f64()?, r.f64()?, r.f64()?, r.f64()?);
        let round = r.u8()? == 1;
        layers.push(Layer { thickness, gap_before, width, height, round, material: material(&r.str()?) });
    }
    let kind = match r.u8()? { 0 => OutcomeKind::Perforation, 1 => OutcomeKind::PartialPenetration, _ => OutcomeKind::Rejected };
    let (v_residual, penetration, energy_absorbed_pct, target_ejected_pct, proj_behind_pct, energy_error_pct) = (r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?, r.f64()?);
    let outcome = Outcome { kind, v_residual, penetration, energy_absorbed_pct, target_ejected_pct, proj_behind_pct, energy_error_pct };
    let gl = r.u64()? as usize;
    let g = zstd::bulk::decompress(r.take(gl)?, 1 << 31).ok()?;
    let mut gr = R(&g);
    let n = gr.u64()? as usize;
    let parts = gr.take(n)?.to_vec();
    let surface: Vec<bool> = gr.take(n)?.iter().map(|&b| b != 0).collect();
    let mut vec3 = |gr: &mut R| -> Option<Vec<[f32; 3]>> {
        let raw = gr.take(n * 12)?;
        Some(raw.chunks_exact(12).map(|c| [f32::from_le_bytes(c[0..4].try_into().unwrap()), f32::from_le_bytes(c[4..8].try_into().unwrap()), f32::from_le_bytes(c[8..12].try_into().unwrap())]).collect())
    };
    let normals = vec3(&mut gr)?;
    let ref_pos = vec3(&mut gr)?;
    let mut rest: &[u8] = r.0;
    let mut frames = Vec::with_capacity(nframes);
    for _ in 0..nframes {
        frames.push(Arc::new(PackedFrame::read_from(&mut rest)?));
    }
    Some(SaveData {
        title, velocity, obliquity_deg, dx, scale,
        design: Design { projectile: Projectile { profile, material: pm, core }, layers },
        n_phys, dt, steps, wall, t_end, back_x, sym, outcome, parts, surface, normals, ref_pos, frames,
    })
}
