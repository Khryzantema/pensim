//! Adaptador do núcleo de elementos finitos para a interface do app (mesma cara do `Sim` antigo):
//! quadros por nó, resultado do disparo, progresso e "parar aqui".
use super::build::{build, Setup};
use super::{Fem, FemModel};
use crate::config::Config;
use crate::sim::{Frame, Outcome, OutcomeKind};
use crate::store::PackedFrame;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub struct FemSim {
    pub fem: Fem,
    pub cfg: Config,
    pub frames: Vec<PackedFrame>,
    n_proj: u8,
    back_x: f64,
    dir: [f64; 3],
    sym: u8,
    /// fator das energias do modelo reduzido para o modelo inteiro
    gfac: f64,
    /// peça de cada nó (a de menor índice entre os elementos dele: projétil vence)
    node_part: Vec<u8>,
    surface: Vec<bool>,
    normals: Vec<[f32; 3]>,
    ref_pos: Vec<[f32; 3]>,
    t_end: f64,
    frame_dt: f64,
    next_frame: f64,
    /// quadros extras durante o impacto: intervalo mínimo e estado do último quadro gravado
    fast_dt: f64,
    last_rec_t: f64,
    last_ke_p: f64,
    ke0: f64,
    m_target: f64,
    /// face da frente do alvo e o ponto mais fundo já alcançado pela parte inteira do projétil
    /// (sem fragmentos soltos), ao longo de toda a simulação
    front_x: f64,
    max_tip: f64,
}

impl FemSim {
    pub fn new(cfg: Config) -> Result<FemSim, String> {
        let Setup { mesh, mats, v0, n_proj, back_x, dir, sym } = build(&cfg)?;
        let n = mesh.n_nodes();
        let mut node_part = vec![u8::MAX; n];
        for (e, h) in mesh.hex.iter().enumerate() {
            for &k in h {
                node_part[k as usize] = node_part[k as usize].min(mesh.part[e]);
            }
        }
        // superfície inicial: nós de faces com um só elemento (fora dos planos de simetria) e a
        // normal média dessas faces
        let mut faces: std::collections::HashMap<[u32; 4], (u32, [u32; 4])> = std::collections::HashMap::new();
        for h in &mesh.hex {
            for f in &super::mesh::FACES {
                let ids = f.map(|l| h[l]);
                let mut k = ids;
                k.sort_unstable();
                faces.entry(k).or_insert((0, ids)).0 += 1;
            }
        }
        let mut surface = vec![false; n];
        let mut nrm = vec![[0.0f64; 3]; n];
        for (_, (c, ids)) in faces {
            if c != 1 {
                continue;
            }
            let common = ids.iter().fold(0xFFu8, |a, &k| a & mesh.sym[k as usize]);
            if common & 6 != 0 && common != 7 {
                continue;
            }
            let p = ids.map(|k| mesh.x[k as usize]);
            let d1 = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
            let d2 = [p[3][0] - p[1][0], p[3][1] - p[1][1], p[3][2] - p[1][2]];
            let cr = [d1[1] * d2[2] - d1[2] * d2[1], d1[2] * d2[0] - d1[0] * d2[2], d1[0] * d2[1] - d1[1] * d2[0]];
            for &k in &ids {
                surface[k as usize] = true;
                for a in 0..3 {
                    nrm[k as usize][a] += cr[a];
                }
            }
        }
        let normals = nrm
            .iter()
            .map(|v| {
                let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if l > 0.0 { [(v[0] / l) as f32, (v[1] / l) as f32, (v[2] / l) as f32] } else { [0.0; 3] }
            })
            .collect();
        let ref_pos = mesh.x.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect();
        let mut model = FemModel::new(mesh, mats, v0);
        model.contact = true;
        model.friction = 0.1;
        let fem = Fem::new(model);
        let gfac = match sym {
            0 => 1.0,
            3 => 4.0,
            _ => 2.0,
        };
        let t_end = cfg.t_end();
        let frame_dt = t_end / cfg.n_frames.max(2) as f64;
        let fast_dt = frame_dt / 10.0;
        let m_target = (0..n).filter(|&i| node_part[i] >= n_proj).map(|i| fem.masses()[i]).sum();
        let front_x = (0..n).filter(|&i| node_part[i] >= n_proj).map(|i| fem.x[i][0]).fold(f64::MAX, f64::min);
        let mut s = FemSim {
            ke0: fem.e0,
            fem,
            cfg,
            frames: Vec::new(),
            n_proj,
            back_x,
            dir,
            sym,
            gfac,
            node_part,
            surface,
            normals,
            ref_pos,
            t_end,
            frame_dt,
            next_frame: 0.0,
            fast_dt,
            last_rec_t: f64::NEG_INFINITY,
            last_ke_p: 0.0,
            m_target,
            front_x,
            max_tip: f64::MIN,
        };
        s.record();
        Ok(s)
    }

    pub fn hex(&self) -> &[[u32; 8]] {
        &self.fem.mesh.hex
    }
    pub fn n_physical(&self) -> usize {
        self.fem.n_nodes()
    }
    pub fn symmetry(&self) -> u8 {
        self.sym
    }
    pub fn normals(&self) -> Vec<[f32; 3]> {
        self.normals.clone()
    }
    pub fn plate_back(&self) -> f64 {
        self.back_x
    }
    pub fn part_ids(&self) -> Vec<u8> {
        self.node_part.clone()
    }
    pub fn surface(&self) -> Vec<bool> {
        self.surface.clone()
    }
    pub fn ref_positions(&self) -> Vec<[f32; 3]> {
        self.ref_pos.clone()
    }
    pub fn t_end(&self) -> f64 {
        self.t_end
    }
    pub fn time(&self) -> f64 {
        self.fem.time
    }
    pub fn dt(&self) -> f64 {
        self.fem.dt
    }
    pub fn steps(&self) -> u64 {
        self.fem.steps
    }
    pub fn done(&self) -> bool {
        self.fem.time >= self.t_end
    }
    pub fn progress(&self) -> f64 {
        (self.fem.time / self.t_end).clamp(0.0, 1.0)
    }

    pub fn advance_for(&mut self, budget: Duration) {
        let t0 = Instant::now();
        while !self.done() && t0.elapsed() < budget {
            self.fem.step();
            if !(self.fem.dt.is_finite() && self.fem.energy.kinetic.is_finite()) {
                // instabilidade: encerra com o que tem em vez de travar o app
                self.t_end = self.fem.time;
                break;
            }
            if self.fem.time >= self.next_frame || self.done() {
                self.record();
            } else if self.fem.time - self.last_rec_t >= self.fast_dt {
                // durante o impacto (o projétil perde energia rápido) grava um quadro a cada
                // `fast_dt`: a repetição mostra o impacto em câmera lenta e o resto no ritmo normal
                let ke = self.proj_ke();
                if (ke - self.last_ke_p).abs() > 0.002 * self.ke0.max(1e-30) {
                    self.record();
                }
            }
        }
    }

    /// Encerra agora (botão "Parar aqui").
    pub fn finish_now(&mut self) {
        self.t_end = self.fem.time;
        self.record();
    }

    fn proj_ke(&self) -> f64 {
        let f = &self.fem;
        let m = f.masses();
        (0..f.n_nodes())
            .filter(|&i| self.node_part[i] < self.n_proj)
            .map(|i| {
                let v = f.v[i];
                0.5 * m[i] * (v[0] * v[0] + v[1] * v[1] + v[2] * v[2])
            })
            .sum()
    }

    fn record(&mut self) {
        let f = &self.fem;
        let [vm, pr, ep, temp, dmg] = f.node_fields();
        let speed: Vec<f32> = f.v.iter().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt() as f32).collect();
        let m = f.masses();
        let (mut ke_p, mut ke_t) = (0.0, 0.0);
        for i in 0..f.n_nodes() {
            // penetração: só nós do projétil ainda presos a um elemento (dano < 1)
            if self.node_part[i] < self.n_proj && dmg[i] < 1.0 {
                self.max_tip = self.max_tip.max(f.x[i][0]);
            }
            let v = f.v[i];
            let k = 0.5 * m[i] * (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
            if self.node_part[i] < self.n_proj {
                ke_p += k;
            } else {
                ke_t += k;
            }
        }
        let en = f.energy;
        let frame = Frame {
            time: f.time,
            pos: f.x.iter().map(|p| [p[0] as f32, p[1] as f32, p[2] as f32]).collect(),
            speed,
            von_mises: vm,
            pressure: pr,
            plastic: ep,
            temp,
            damage: dmg,
            ke_proj: ke_p * self.gfac,
            ke_target: ke_t * self.gfac,
            vx_proj: self.residual_velocity(),
            e_elastic: (en.internal + en.hourglass) * self.gfac,
            e_contact: en.contact * self.gfac,
            e_plastic: 0.0,
            e_fracture: 0.0,
            alive: Some(Arc::new(f.alive_bits())),
        };
        // comprimido (posição em 16 bits, campos em 8): com os quadros extras do impacto o
        // número de quadros cresce, e cru não caberia na memória do celular
        self.frames.push(PackedFrame::pack(&frame, true));
        self.last_rec_t = f.time;
        self.last_ke_p = ke_p;
        while self.next_frame <= f.time + 1e-15 {
            self.next_frame += self.frame_dt;
        }
    }

    /// Velocidade ao longo da trajetória do que saiu pela face traseira (se > 5% da massa do
    /// projétil), senão a média do projétil todo.
    pub fn residual_velocity(&self) -> f64 {
        let f = &self.fem;
        let m = f.masses();
        let d = self.dir;
        let (mut mp, mut mb, mut pv, mut pvb) = (0.0, 0.0, 0.0, 0.0);
        for i in 0..f.n_nodes() {
            if self.node_part[i] >= self.n_proj {
                continue;
            }
            let v = f.v[i][0] * d[0] + f.v[i][2] * d[2];
            mp += m[i];
            pv += m[i] * v;
            if f.x[i][0] > self.back_x {
                mb += m[i];
                pvb += m[i] * v;
            }
        }
        if mb > 0.05 * mp { pvb / mb } else { pv / mp.max(1e-30) }
    }

    pub fn outcome(&self) -> Outcome {
        let f = &self.fem;
        let m = f.masses();
        let d = self.dir;
        let (mut mp, mut mb, mut tip, mut ke_p) = (0.0, 0.0, f64::MIN, 0.0);
        let mut ejected = 0.0;
        let mut pv = 0.0;
        for i in 0..f.n_nodes() {
            let v = f.v[i];
            if self.node_part[i] < self.n_proj {
                mp += m[i];
                pv += m[i] * (v[0] * d[0] + v[2] * d[2]);
                tip = tip.max(f.x[i][0]);
                if f.x[i][0] > self.back_x {
                    mb += m[i];
                }
                ke_p += 0.5 * m[i] * (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]);
            } else if f.x[i][0] > self.back_x + 2.0 * self.cfg.dx {
                ejected += m[i];
            }
        }
        let v_res = self.residual_velocity();
        let behind = mb / mp.max(1e-30);
        let v_cg = pv / mp.max(1e-30);
        let kind = if behind > 0.5 && v_res > 0.0 {
            OutcomeKind::Perforation
        } else if v_cg < 0.0 && behind < 0.05 {
            OutcomeKind::Rejected
        } else if behind > 0.1 && v_res > 0.0 && tip > self.back_x + 2.0 * self.cfg.dx {
            OutcomeKind::Perforation
        } else {
            OutcomeKind::PartialPenetration
        };
        let ke0 = self.ke0.max(1e-30);
        Outcome {
            kind,
            v_residual: v_res,
            penetration: (self.max_tip.max(tip.min(self.front_x)) - self.front_x).clamp(0.0, self.back_x - self.front_x),
            energy_absorbed_pct: 100.0 * (1.0 - ke_p / ke0),
            target_ejected_pct: 100.0 * ejected / self.m_target.max(1e-30),
            proj_behind_pct: 100.0 * behind,
            energy_error_pct: 100.0 * f.energy.balance(f.e0) / ke0,
        }
    }
}
