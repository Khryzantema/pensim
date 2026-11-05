//! Biblioteca de munições famosas (dimensões aproximadas, de fontes públicas).
//!
//! Munições grandes não cabem na malha de um celular em tamanho real, então o modelo é
//! gerado em **escala** `s` (todas as dimensões × s, mesma velocidade). Sem efeitos de taxa
//! de deformação, isso preserva a resposta física (similaridade geométrica); as energias
//! escalam com `s³`. Camisas finas (cobre/latão) ficam abaixo da resolução da malha e são
//! ignoradas: o corpo é modelado com o material principal e, quando existe, um núcleo.
use crate::config::{Config, Core, Design, Layer, Nose, Projectile};
use crate::material::LIBRARY;

pub struct AmmoCore {
    /// início e fim do núcleo, como fração do comprimento (a partir da ponta)
    pub a0: f64,
    pub a1: f64,
    /// raio do núcleo como fração do raio máximo
    pub r_frac: f64,
    /// índice em `LIBRARY`
    pub mat: usize,
}

pub struct Ammo {
    pub name: &'static str,
    pub group: &'static str,
    pub note: &'static str,
    pub caliber_mm: f64,
    pub length_mm: f64,
    pub mass_g: f64,
    pub velocity: f64,
    pub nose: Nose,
    /// índice em `LIBRARY` do corpo
    pub body_mat: usize,
    pub core: Option<AmmoCore>,
    pub plate_mat: usize,
    pub plate_mm: f64,
    pub scale: f64,
}

const PB: usize = 6;
const RHA: usize = 0;
const MILD: usize = 1;
const AL: usize = 2;
const W: usize = 4;
const DU: usize = 9;
const FILL: usize = 10;
const AP: usize = 11;

macro_rules! core {
    ($a0:expr, $a1:expr, $r:expr, $m:expr) => {
        Some(AmmoCore { a0: $a0, a1: $a1, r_frac: $r, mat: $m })
    };
}

pub const AMMO: &[Ammo] = &[
    Ammo { name: "9×19 mm Parabellum FMJ", group: "Armas portáteis", note: "Projétil de pistola; camisa de cobre não resolvida (corpo de chumbo).",
        caliber_mm: 9.0, length_mm: 15.0, mass_g: 8.0, velocity: 360.0, nose: Nose::Hemispherical, body_mat: PB, core: None, plate_mat: AL, plate_mm: 8.0, scale: 1.0 },
    Ammo { name: "5,56×45 mm M855 (SS109)", group: "Armas portáteis", note: "Penetrador de aço na ponta sobre núcleo de chumbo.",
        caliber_mm: 5.7, length_mm: 23.0, mass_g: 4.0, velocity: 940.0, nose: Nose::Ogive, body_mat: PB, core: core!(0.05, 0.45, 0.62, AP), plate_mat: MILD, plate_mm: 6.0, scale: 1.5 },
    Ammo { name: "7,62×51 mm NATO M80 (ball)", group: "Armas portáteis", note: "Bala comum de fuzil; corpo de chumbo (camisa não resolvida).",
        caliber_mm: 7.8, length_mm: 28.5, mass_g: 9.7, velocity: 840.0, nose: Nose::Ogive, body_mat: PB, core: None, plate_mat: MILD, plate_mm: 8.0, scale: 1.1 },
    Ammo { name: "7,62×51 mm NATO M62 (traçante)", group: "Armas portáteis", note: "Traçante: composição de baixa densidade na traseira (aprox.).",
        caliber_mm: 7.8, length_mm: 29.0, mass_g: 9.4, velocity: 855.0, nose: Nose::Ogive, body_mat: PB, core: core!(0.55, 0.97, 0.6, FILL), plate_mat: MILD, plate_mm: 8.0, scale: 1.1 },
    Ammo { name: "7,62×51 mm NATO M61 (AP)", group: "Armas portáteis", note: "Perfurante: núcleo de aço endurecido.",
        caliber_mm: 7.8, length_mm: 29.5, mass_g: 9.7, velocity: 840.0, nose: Nose::Ogive, body_mat: PB, core: core!(0.05, 0.82, 0.62, AP), plate_mat: RHA, plate_mm: 10.0, scale: 1.1 },
    Ammo { name: "7,62×54R B-32 (API)", group: "Armas portáteis", note: "Perfurante-incendiária soviética; núcleo de aço.",
        caliber_mm: 7.9, length_mm: 32.0, mass_g: 10.4, velocity: 820.0, nose: Nose::Ogive, body_mat: PB, core: core!(0.05, 0.85, 0.65, AP), plate_mat: RHA, plate_mm: 12.0, scale: 1.1 },
    Ammo { name: ".50 BMG M2 AP", group: "Armas portáteis", note: "Metralhadora pesada Browning M2; núcleo de aço.",
        caliber_mm: 12.95, length_mm: 57.0, mass_g: 46.0, velocity: 890.0, nose: Nose::Ogive, body_mat: PB, core: core!(0.05, 0.80, 0.66, AP), plate_mat: RHA, plate_mm: 20.0, scale: 0.7 },
    Ammo { name: "14,5×114 mm B-32 (KPV)", group: "Armas portáteis", note: "Antimaterial soviético; núcleo de aço.",
        caliber_mm: 14.5, length_mm: 63.0, mass_g: 64.0, velocity: 976.0, nose: Nose::Ogive, body_mat: PB, core: core!(0.05, 0.82, 0.68, AP), plate_mat: RHA, plate_mm: 25.0, scale: 0.62 },
    Ammo { name: "75 mm PzGr 39/42 (Panther KwK 42 L/70)", group: "Canhões históricos", note: "APCBC alemão: corpo de aço com cavidade de carga. Escala reduzida.",
        caliber_mm: 75.0, length_mm: 310.0, mass_g: 6800.0, velocity: 935.0, nose: Nose::Ogive, body_mat: AP, core: core!(0.45, 0.95, 0.55, FILL), plate_mat: RHA, plate_mm: 100.0, scale: 0.12 },
    Ammo { name: "88 mm PzGr 39/43 (KwK 43 L/71)", group: "Canhões históricos", note: "APCBC do Tiger II / Nashorn. Escala reduzida.",
        caliber_mm: 88.0, length_mm: 380.0, mass_g: 10200.0, velocity: 1000.0, nose: Nose::Ogive, body_mat: AP, core: core!(0.45, 0.95, 0.55, FILL), plate_mat: RHA, plate_mm: 140.0, scale: 0.10 },
    Ammo { name: "128 mm PzGr 43 (Pak 44 / Jagdtiger)", group: "Canhões históricos", note: "O maior projétil AP alemão em serviço, contra 300 mm de aço. Escala reduzida (×0,12 ≈ 10 nós no diâmetro).",
        caliber_mm: 128.0, length_mm: 600.0, mass_g: 26200.0, velocity: 920.0, nose: Nose::Ogive, body_mat: AP, core: core!(0.45, 0.95, 0.55, FILL), plate_mat: RHA, plate_mm: 300.0, scale: 0.12 },
    Ammo { name: "120 mm M829A1 APFSDS (DU)", group: "Penetradores modernos", note: "Dardo de urânio empobrecido (sabot ignorado). Escala reduzida.",
        caliber_mm: 22.0, length_mm: 700.0, mass_g: 4600.0, velocity: 1575.0, nose: Nose::Conical, body_mat: DU, core: None, plate_mat: RHA, plate_mm: 150.0, scale: 0.3 },
    Ammo { name: "125 mm 3BM42 'Mango' APFSDS (W)", group: "Penetradores modernos", note: "Dardo de tungstênio soviético/russo (sabot ignorado). Escala reduzida.",
        caliber_mm: 24.0, length_mm: 560.0, mass_g: 4850.0, velocity: 1700.0, nose: Nose::Conical, body_mat: W, core: None, plate_mat: RHA, plate_mm: 150.0, scale: 0.28 },
];

impl Ammo {
    /// Configuração do modelo na escala `scale` (todas as dimensões × scale).
    pub fn config(&self, scale: f64) -> Config {
        let s = scale;
        let d = self.caliber_mm * 1e-3 * s;
        let l = self.length_mm * 1e-3 * s;
        let profile = self.nose.profile(d, l);
        let core = self.core.as_ref().map(|c| {
            let r = c.r_frac * d / 2.0;
            Core { profile: vec![[c.a0 * l, r], [c.a1 * l, r]], material: LIBRARY[c.mat].clone() }
        });
        // alvos de projéteis grandes: 5 calibres de largura (bem menos nós, resultado equivalente)
        let width = ((if self.caliber_mm >= 70.0 { 5.0 } else { 7.0 }) * d).max(0.04);
        let cfg = Config {
            dx: 0.0015,
            horizon_m: 2.5,
            design: Design {
                projectile: Projectile { profile, material: LIBRARY[self.body_mat].clone(), core },
                layers: vec![Layer {
                    thickness: self.plate_mm * 1e-3 * s,
                    gap_before: 0.0,
                    width,
                    height: width,
                    round: false,
                    material: LIBRARY[self.plate_mat].clone(),
                }],
            },
            velocity: self.velocity,
            obliquity_deg: 0.0,
            clamp_edges: true,
            time_factor: 1.0,
            n_frames: 100,
            symmetry: true,
            sleep_ratio: 0.03,
            noise: 0.15,
            jitter: 0.05,
            ignore_memory: false,
        };
        cfg
    }

    /// Massa do modelo em escala (kg) = massa real × s³.
    pub fn scaled_mass_g(&self, scale: f64) -> f64 {
        self.mass_g * scale.powi(3)
    }
}
