//! Armazenamento de quadros. Por padrão ficam crus (rápido). Se a simulação estourar a memória,
//! passam a ser guardados quantizados (posição 16 bits, campos 8 bits), em planos de bytes com
//! delta e comprimidos com zstd; só o quadro exibido é descomprimido.
use crate::sim::Frame;
use std::sync::Arc;

/// Quadro comprimido. Os escalares (energias, tempo) ficam em claro para os gráficos.
pub struct PackedFrame {
    pub time: f64,
    pub ke_proj: f64,
    pub ke_target: f64,
    pub vx_proj: f64,
    pub e_elastic: f64,
    pub e_contact: f64,
    pub e_plastic: f64,
    pub e_fracture: f64,
    /// elementos vivos (núcleo de elementos finitos); não vai para o arquivo salvo
    pub alive: Option<Arc<Vec<u8>>>,
    kind: Kind,
}

enum Kind {
    Raw(Arc<Frame>),
    Zstd { n: usize, pos_min: [f32; 3], pos_step: [f32; 3], f_min: [f32; 6], f_step: [f32; 6], data: Vec<u8> },
}

fn range(v: impl Iterator<Item = f32>) -> (f32, f32) {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for x in v {
        if x.is_finite() {
            lo = lo.min(x);
            hi = hi.max(x);
        }
    }
    if lo > hi {
        (0.0, 0.0)
    } else {
        (lo, hi)
    }
}

impl PackedFrame {
    pub fn pack(f: &Frame, compress: bool) -> PackedFrame {
        if !compress {
            return PackedFrame {
                time: f.time,
                ke_proj: f.ke_proj,
                ke_target: f.ke_target,
                vx_proj: f.vx_proj,
                e_elastic: f.e_elastic,
                e_contact: f.e_contact,
                e_plastic: f.e_plastic,
                e_fracture: f.e_fracture,
                alive: f.alive.clone(),
                kind: Kind::Raw(Arc::new(f.clone())),
            };
        }
        let n = f.pos.len();
        let mut raw: Vec<u8> = Vec::with_capacity(n * 12);
        let (mut pos_min, mut pos_step) = ([0.0f32; 3], [1.0f32; 3]);
        for a in 0..3 {
            let (lo, hi) = range(f.pos.iter().map(|p| p[a]));
            pos_min[a] = lo;
            pos_step[a] = if hi > lo { (hi - lo) / 65535.0 } else { 1.0 };
            let mut q: Vec<u16> = Vec::with_capacity(n);
            let mut prev = 0u16;
            for p in &f.pos {
                let v = if p[a].is_finite() { ((p[a] - lo) / pos_step[a]).round().clamp(0.0, 65535.0) as u16 } else { 0 };
                q.push(v.wrapping_sub(prev)); // delta: nós vizinhos na ordem de Morton ficam próximos
                prev = v;
            }
            raw.extend(q.iter().map(|v| (v >> 8) as u8)); // plano de bytes altos
            raw.extend(q.iter().map(|v| *v as u8)); // plano de bytes baixos
        }
        let fields: [&Vec<f32>; 6] = [&f.speed, &f.von_mises, &f.pressure, &f.plastic, &f.temp, &f.damage];
        let (mut f_min, mut f_step) = ([0.0f32; 6], [1.0f32; 6]);
        for (k, fl) in fields.iter().enumerate() {
            let (lo, hi) = range(fl.iter().cloned());
            f_min[k] = lo;
            f_step[k] = if hi > lo { (hi - lo) / 255.0 } else { 1.0 };
            raw.extend(fl.iter().map(|&x| if x.is_finite() { ((x - lo) / f_step[k]).round().clamp(0.0, 255.0) as u8 } else { 0 }));
        }
        let data = zstd::bulk::compress(&raw, 3).unwrap_or(raw);
        PackedFrame {
            time: f.time,
            ke_proj: f.ke_proj,
            ke_target: f.ke_target,
            vx_proj: f.vx_proj,
            e_elastic: f.e_elastic,
            e_contact: f.e_contact,
            e_plastic: f.e_plastic,
            e_fracture: f.e_fracture,
            alive: f.alive.clone(),
            kind: Kind::Zstd { n, pos_min, pos_step, f_min, f_step, data },
        }
    }

    /// Serializa o quadro (sempre comprimido) para salvar em arquivo.
    pub fn write_to(&self, out: &mut Vec<u8>) {
        let packed;
        let me: &PackedFrame = if self.is_compressed() {
            self
        } else {
            packed = PackedFrame::pack(&self.unpack(), true);
            &packed
        };
        for v in [me.time, me.ke_proj, me.ke_target, me.vx_proj, me.e_elastic, me.e_contact, me.e_plastic, me.e_fracture] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        if let Kind::Zstd { n, pos_min, pos_step, f_min, f_step, data } = &me.kind {
            out.extend_from_slice(&(*n as u64).to_le_bytes());
            for v in pos_min.iter().chain(pos_step).chain(f_min).chain(f_step) {
                out.extend_from_slice(&v.to_le_bytes());
            }
            out.extend_from_slice(&(data.len() as u64).to_le_bytes());
            out.extend_from_slice(data);
        }
    }

    /// Lê um quadro serializado por `write_to`, avançando o cursor.
    pub fn read_from(r: &mut &[u8]) -> Option<PackedFrame> {
        fn take<'a>(r: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
            if r.len() < n {
                return None;
            }
            let (a, b) = r.split_at(n);
            *r = b;
            Some(a)
        }
        let f64n = |r: &mut &[u8]| -> Option<f64> { Some(f64::from_le_bytes(take(r, 8)?.try_into().ok()?)) };
        let f32n = |r: &mut &[u8]| -> Option<f32> { Some(f32::from_le_bytes(take(r, 4)?.try_into().ok()?)) };
        let mut sc = [0.0f64; 8];
        for v in sc.iter_mut() {
            *v = f64n(r)?;
        }
        let n = u64::from_le_bytes(take(r, 8)?.try_into().ok()?) as usize;
        let (mut pos_min, mut pos_step, mut f_min, mut f_step) = ([0.0f32; 3], [0.0f32; 3], [0.0f32; 6], [0.0f32; 6]);
        for v in pos_min.iter_mut().chain(pos_step.iter_mut()).chain(f_min.iter_mut()).chain(f_step.iter_mut()) {
            *v = f32n(r)?;
        }
        let len = u64::from_le_bytes(take(r, 8)?.try_into().ok()?) as usize;
        let data = take(r, len)?.to_vec();
        Some(PackedFrame {
            time: sc[0],
            ke_proj: sc[1],
            ke_target: sc[2],
            vx_proj: sc[3],
            e_elastic: sc[4],
            e_contact: sc[5],
            e_plastic: sc[6],
            e_fracture: sc[7],
            alive: None,
            kind: Kind::Zstd { n, pos_min, pos_step, f_min, f_step, data },
        })
    }

    /// Bytes ocupados pelo quadro (aproximado).
    pub fn size_bytes(&self) -> usize {
        match &self.kind {
            Kind::Raw(f) => f.pos.len() * 36,
            Kind::Zstd { data, .. } => data.len(),
        }
    }

    pub fn is_compressed(&self) -> bool {
        matches!(self.kind, Kind::Zstd { .. })
    }

    /// Quadro completo (cru: compartilha o mesmo `Arc`; comprimido: descomprime e dequantiza).
    pub fn unpack(&self) -> Arc<Frame> {
        let (n, pos_min, pos_step, f_min, f_step, data) = match &self.kind {
            Kind::Raw(f) => return f.clone(),
            Kind::Zstd { n, pos_min, pos_step, f_min, f_step, data } => (*n, *pos_min, *pos_step, *f_min, *f_step, data),
        };
        let expected = n * 12;
        let raw = zstd::bulk::decompress(data, expected).unwrap_or_else(|_| data.clone());
        let mut pos = vec![[0.0f32; 3]; n];
        for a in 0..3 {
            let hi = &raw[a * 2 * n..a * 2 * n + n];
            let lo = &raw[a * 2 * n + n..a * 2 * n + 2 * n];
            let mut prev = 0u16;
            for i in 0..n {
                let v = prev.wrapping_add(((hi[i] as u16) << 8) | lo[i] as u16);
                prev = v;
                pos[i][a] = pos_min[a] + v as f32 * pos_step[a];
            }
        }
        let mut fl: [Vec<f32>; 6] = Default::default();
        for k in 0..6 {
            let base = 6 * n + k * n;
            fl[k] = raw[base..base + n].iter().map(|&b| f_min[k] + b as f32 * f_step[k]).collect();
        }
        let [speed, von_mises, pressure, plastic, temp, damage] = fl;
        Arc::new(Frame {
            time: self.time,
            pos,
            speed,
            von_mises,
            pressure,
            plastic,
            temp,
            damage,
            ke_proj: self.ke_proj,
            ke_target: self.ke_target,
            vx_proj: self.vx_proj,
            e_elastic: self.e_elastic,
            e_contact: self.e_contact,
            e_plastic: self.e_plastic,
            e_fracture: self.e_fracture,
            alive: self.alive.clone(),
        })
    }
}
