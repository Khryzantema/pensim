//! Materiais do núcleo de elementos finitos: Johnson-Cook (resistência e dano) + equação de estado.
//!
//! Os números ficam na tabela, não são escolhidos pelo nome do material.

/// Resistência de Johnson-Cook: σy = (A + B εp^n)(1 + C ln ε̇*)(1 − T*^m), ε̇* = ε̇p / ε̇0.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JcStrength {
    pub a: f64,
    pub b: f64,
    pub n: f64,
    pub c: f64,
    pub m: f64,
    /// taxa de referência [1/s]
    pub eps_dot0: f64,
}

/// Dano de Johnson-Cook: εf = (D1 + D2 e^{D3 σ*})(1 + D4 ln ε̇*)(1 + D5 T*).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JcDamage {
    pub d: [f64; 5],
}

/// Equação de estado (parte volumétrica).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Eos {
    /// p = K μ, μ = V0/V − 1
    Linear { k: f64 },
    /// Mie-Grüneisen com referência de Hugoniot linear us = c0 + s up
    MieGruneisen { c0: f64, s: f64, gamma0: f64 },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FemMat {
    pub name: &'static str,
    pub rho: f64,
    /// módulo de cisalhamento [Pa]
    pub g: f64,
    pub jc: JcStrength,
    pub dmg: Option<JcDamage>,
    pub eos: Eos,
    /// calor específico [J/kg/K]
    pub cp: f64,
    pub t_room: f64,
    pub t_melt: f64,
    /// fração do trabalho plástico que vira calor
    pub taylor_quinney: f64,
    /// ruptura frágil por tensão principal máxima [Pa] (cerâmicas, compósitos)
    pub brittle_stress: Option<f64>,
}

impl FemMat {
    /// Módulo volumétrico de referência (inclinação da EOS em μ = 0).
    pub fn bulk(&self) -> f64 {
        match self.eos {
            Eos::Linear { k } => k,
            Eos::MieGruneisen { c0, .. } => self.rho * c0 * c0,
        }
    }

    /// Material elástico linear isotrópico (só para testes de verificação): escoamento enorme.
    pub fn elastic(name: &'static str, rho: f64, e: f64, nu: f64) -> FemMat {
        let g = e / (2.0 * (1.0 + nu));
        let k = e / (3.0 * (1.0 - 2.0 * nu));
        FemMat {
            name,
            rho,
            g,
            jc: JcStrength { a: 1e30, b: 0.0, n: 1.0, c: 0.0, m: 1.0, eps_dot0: 1.0 },
            dmg: None,
            eos: Eos::Linear { k },
            cp: 500.0,
            t_room: 293.0,
            t_melt: 1e9,
            taylor_quinney: 0.9,
            brittle_stress: None,
        }
    }

    /// Elastoplástico perfeito (sem encruamento, sem taxa, sem amolecimento): para testes.
    pub fn perfectly_plastic(name: &'static str, rho: f64, e: f64, nu: f64, sy: f64) -> FemMat {
        let mut m = FemMat::elastic(name, rho, e, nu);
        m.jc.a = sy;
        m
    }

    /// Tensão de escoamento de Johnson-Cook.
    pub fn yield_stress(&self, eps_p: f64, eps_rate: f64, temp: f64) -> f64 {
        let j = &self.jc;
        let hard = j.a + if j.b > 0.0 && eps_p > 0.0 { j.b * eps_p.powf(j.n) } else { 0.0 };
        let r = eps_rate / j.eps_dot0;
        let rate = if j.c > 0.0 && r > 1.0 { 1.0 + j.c * r.ln() } else { 1.0 };
        let ts = ((temp - self.t_room) / (self.t_melt - self.t_room)).clamp(0.0, 1.0);
        let therm = if ts > 0.0 { 1.0 - ts.powf(j.m) } else { 1.0 };
        hard * rate * therm.max(0.0)
    }

    /// Inclinação do encruamento dσy/dεp (só a parte de B·εp^n).
    pub fn hardening_slope(&self, eps_p: f64) -> f64 {
        let j = &self.jc;
        if j.b <= 0.0 {
            0.0
        } else {
            j.b * j.n * eps_p.max(1e-6).powf(j.n - 1.0)
        }
    }

    /// Pressão (compressão positiva) pela EOS. `mu` = V0/V − 1, `e_vol` = energia interna por
    /// volume inicial [J/m³].
    pub fn pressure(&self, mu: f64, e_vol: f64) -> f64 {
        match self.eos {
            Eos::Linear { k } => k * mu,
            Eos::MieGruneisen { c0, s, gamma0 } => {
                let r0c2 = self.rho * c0 * c0;
                if mu > 0.0 {
                    let den = 1.0 - (s - 1.0) * mu;
                    r0c2 * mu * (1.0 + (1.0 - 0.5 * gamma0) * mu) / (den * den).max(1e-6) + gamma0 * e_vol
                } else {
                    r0c2 * mu + gamma0 * e_vol
                }
            }
        }
    }

    /// Módulo volumétrico tangente aproximado (para o passo de tempo).
    pub fn bulk_tangent(&self, mu: f64) -> f64 {
        match self.eos {
            Eos::Linear { k } => k,
            Eos::MieGruneisen { c0, s, .. } => {
                let k0 = self.rho * c0 * c0;
                if mu > 0.0 {
                    let den = 1.0 - (s - 1.0) * mu;
                    k0 * (1.0 + 2.0 * s * mu) / (den * den * den).max(1e-6)
                } else {
                    k0
                }
            }
        }
    }

    /// Deformação plástica de ruptura de Johnson-Cook (sem a regularização pela malha).
    pub fn failure_strain(&self, triax: f64, eps_rate: f64, temp: f64) -> Option<f64> {
        let d = self.dmg?.d;
        let r = eps_rate / self.jc.eps_dot0;
        let rate = if r > 1.0 { 1.0 + d[3] * r.ln() } else { 1.0 };
        let ts = ((temp - self.t_room) / (self.t_melt - self.t_room)).clamp(0.0, 1.0);
        Some(((d[0] + d[1] * (d[2] * triax).exp()) * rate * (1.0 + d[4] * ts)).max(1e-3))
    }
}

/// Parâmetros de literatura. Fonte principal: Johnson & Cook (1983, 1985); EOS de Hugoniot das
/// tabelas de Marsh (LASL Shock Hugoniot Data, 1980) / Meyers (1994).
/// **A conferir contra os artigos na fase 5** (estão aqui de memória de valores muito citados).
pub mod lib {
    use super::*;

    /// Aço 4340 (Rc 30), Johnson & Cook 1985.
    pub fn steel_4340() -> FemMat {
        FemMat {
            name: "Aço 4340",
            rho: 7830.0,
            g: 77.5e9,
            jc: JcStrength { a: 792e6, b: 510e6, n: 0.26, c: 0.014, m: 1.03, eps_dot0: 1.0 },
            dmg: Some(JcDamage { d: [0.05, 3.44, -2.12, 0.002, 0.61] }),
            eos: Eos::MieGruneisen { c0: 4578.0, s: 1.33, gamma0: 1.67 },
            cp: 477.0,
            t_room: 293.0,
            t_melt: 1793.0,
            taylor_quinney: 0.9,
            brittle_stress: None,
        }
    }

    /// Cobre OFHC, Johnson & Cook 1983/1985.
    pub fn copper_ofhc() -> FemMat {
        FemMat {
            name: "Cobre OFHC",
            rho: 8960.0,
            g: 46e9,
            jc: JcStrength { a: 90e6, b: 292e6, n: 0.31, c: 0.025, m: 1.09, eps_dot0: 1.0 },
            dmg: Some(JcDamage { d: [0.54, 4.89, -3.03, 0.014, 1.12] }),
            eos: Eos::MieGruneisen { c0: 3940.0, s: 1.49, gamma0: 2.02 },
            cp: 383.0,
            t_room: 293.0,
            t_melt: 1356.0,
            taylor_quinney: 0.9,
            brittle_stress: None,
        }
    }

    /// Alumínio 6061-T6 (valores muito usados na literatura de impacto).
    pub fn al_6061_t6() -> FemMat {
        FemMat {
            name: "Alumínio 6061-T6",
            rho: 2703.0,
            g: 26e9,
            jc: JcStrength { a: 324e6, b: 114e6, n: 0.42, c: 0.002, m: 1.34, eps_dot0: 1.0 },
            dmg: Some(JcDamage { d: [-0.77, 1.45, -0.47, 0.0, 1.6] }),
            eos: Eos::MieGruneisen { c0: 5350.0, s: 1.34, gamma0: 2.0 },
            cp: 885.0,
            t_room: 293.0,
            t_melt: 925.0,
            taylor_quinney: 0.9,
            brittle_stress: None,
        }
    }
}
