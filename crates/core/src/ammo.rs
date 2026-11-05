//! Biblioteca de munições famosas, montadas peça por peça a partir de cortes/desenhos técnicos
//! públicos (dimensões aproximadas): camisa, núcleo, enchimento de ponta, capuz perfurante,
//! para-brisa balístico oco, carga explosiva, traçante, cinta de condução.
//!
//! Munições grandes não cabem na malha de um celular em tamanho real, então o modelo é
//! gerado em **escala** `s` (todas as dimensões × s, mesma velocidade). Sem efeitos de taxa
//! de deformação, isso preserva a resposta física (similaridade geométrica); as energias
//! escalam com `s³`. Peças mais finas que um nó (camisas, para-brisas, cintas) são engrossadas
//! até a espessura de um nó na montagem da malha, para não sumirem.
use crate::config::{Config, Design, Fill, Layer, Projectile, Region};
use crate::material::LIBRARY;

const RHA: usize = 0;
const MILD: usize = 1;
const AL: usize = 2;
const W: usize = 4;
const CU: usize = 5;
const PB: usize = 6;
const DU: usize = 9;
const FILL: usize = 10;
const AP: usize = 11;
const WC: usize = 12;
const GILD: usize = 13;
const CAP: usize = 14;

type P = Vec<[f64; 2]>;

/// Projétil montado, em milímetros reais.
pub struct Built {
    pub profile: P,
    pub body: usize,
    pub regions: Vec<Region>,
}

pub struct Ammo {
    pub name: &'static str,
    pub group: &'static str,
    pub note: &'static str,
    pub caliber_mm: f64,
    pub length_mm: f64,
    pub mass_g: f64,
    pub velocity: f64,
    pub build: fn() -> Built,
    pub plate_mat: usize,
    pub plate_mm: f64,
    pub scale: f64,
}

// ---------------------------------------------------------------- geometria (mm)

/// Ogiva tangente com ponta em `a0` (r = 0) chegando ao raio `r` em `a0 + len`.
fn ogive(a0: f64, len: f64, r: f64) -> P {
    let rho = (r * r + len * len) / (2.0 * r);
    (0..=14)
        .map(|k| {
            let z = len * k as f64 / 14.0;
            [a0 + z, ((rho * rho - (len - z) * (len - z)).max(0.0).sqrt() - (rho - r)).max(0.0)]
        })
        .collect()
}

/// Nariz rombudo: face plana de raio `rf` em `a0`, arredondando (elipse) até `r` em `a0 + len`.
fn blunt(a0: f64, len: f64, rf: f64, r: f64) -> P {
    let mut v = vec![[a0, 0.0]];
    for k in 0..=10 {
        let t = k as f64 / 10.0;
        v.push([a0 + 1e-6 + len * t, rf + (r - rf) * (1.0 - (1.0 - t) * (1.0 - t)).sqrt()]);
    }
    v
}

fn with(mut p: P, rest: &[[f64; 2]]) -> P {
    p.extend_from_slice(rest);
    p
}

fn radius(p: &P, a: f64) -> f64 {
    crate::config::profile_radius(p, a)
}

/// Superfície interna de uma casca de espessura `t` (amostrada).
fn shrink(p: &P, t: f64) -> P {
    let (a0, a1) = (p[0][0], p.last().unwrap()[0]);
    (0..=40).map(|k| a0 + (a1 - a0) * k as f64 / 40.0).map(|a| [a, (radius(p, a) - t).max(0.0)]).collect()
}

/// Trecho [a0, a1] de um perfil (amostrado).
fn clip(p: &P, a0: f64, a1: f64) -> P {
    (0..=24).map(|k| a0 + (a1 - a0) * k as f64 / 24.0).map(|a| [a, radius(p, a).max(0.0)]).collect()
}

fn mat(i: usize) -> Fill {
    Fill::Mat(LIBRARY[i].clone())
}

fn reg(name: &str, outer: P, inner: P, fill: Fill) -> Region {
    Region { name: name.into(), outer, inner, fill, poly: vec![] }
}

fn cyl(a0: f64, a1: f64, r: f64) -> P {
    vec![[a0, r], [a1, r]]
}

/// Bala de arma portátil: ogiva + cilindro + base cônica (boat-tail), camisa por fora.
fn bullet(d: f64, l: f64, ogive_len: f64, bt_len: f64, bt_r: f64, jacket: f64, jacket_mat: usize, body: usize, inside: Vec<Region>) -> Built {
    let r = d / 2.0;
    let mut prof = ogive(0.0, ogive_len, r);
    if bt_len > 0.0 {
        prof.push([l - bt_len, r]);
        prof.push([l, bt_r]);
    } else {
        prof.push([l, r]);
    }
    let mut regions = inside;
    // a camisa vem por último: na fronteira ela prevalece sobre o núcleo
    regions.push(reg("Camisa", prof.clone(), shrink(&prof, jacket), mat(jacket_mat)));
    Built { profile: prof, body, regions }
}

/// Medidas de um projétil de canhão perfurante, em calibres (D) a partir da ponta.
struct Shell {
    d: f64,
    l: f64,
    /// para-brisa balístico (oco): comprimento; 0 = sem para-brisa
    ws_len: f64,
    ws_mat: usize,
    /// capuz perfurante: ponta, raio da face plana, onde chega ao calibre, fim da saia; ws/cap 0 = sem
    cap: Option<(f64, f64, f64, f64)>,
    /// nariz do corpo: ponta, comprimento; rombudo (raio da face) ou ogival
    nose_tip: f64,
    nose_len: f64,
    nose_flat: f64,
    /// carga explosiva [g] (0 = sem); raio da cavidade em calibres
    he_g: f64,
    he_r: f64,
    /// base cônica: comprimento e raio final (calibres)
    bt: (f64, f64),
    body: usize,
}

fn shell(s: Shell) -> Built {
    let d = s.d;
    let r = d / 2.0;
    let l = s.l;
    let nose: P = if s.nose_flat > 0.0 { blunt(s.nose_tip * d, s.nose_len * d, s.nose_flat * d, r) } else { ogive(s.nose_tip * d, s.nose_len * d, r) };
    let ws = s.ws_len * d;
    let mut prof: P = if ws > 0.0 { ogive(0.0, ws, r) } else { nose.clone() };
    let bt_a = l - s.bt.0 * d;
    prof.push([bt_a, r]);
    prof.push([l, s.bt.1 * d]);
    let wall = 0.03 * d; // para-brisa de chapa fina
    let mut regions = Vec::new();
    if ws > 0.0 {
        let wsp = clip(&prof, 0.0, ws);
        regions.push(reg("Interior do para-brisa", shrink(&wsp, wall), vec![], Fill::Void));
        // o corpo volta a valer dentro do contorno do nariz
        regions.push(reg("Nariz do corpo", with(nose.clone(), &[[ws + 0.6 * d, r]]), vec![], Fill::Body));
    }
    if let Some((tip, flat, full, end)) = s.cap {
        let outer = with(blunt(tip * d, (full - tip) * d, flat * d, r), &[[end * d, r]]);
        regions.push(reg("Capuz perfurante", outer, nose.clone(), mat(CAP)));
    }
    if ws > 0.0 {
        let wsp = clip(&prof, 0.0, ws);
        regions.push(reg("Para-brisa balístico", wsp.clone(), shrink(&wsp, wall), mat(s.ws_mat)));
    }
    if s.he_g > 0.0 {
        // volume da carga (densidade ~1,65 g/cm³) num cilindro de raio he_r·D, perto da base
        let rc = s.he_r * d;
        let len = (s.he_g / 1.65 * 1000.0 / (std::f64::consts::PI * rc * rc)).max(0.2 * d);
        let a1 = l - 0.40 * d;
        regions.push(reg("Carga explosiva", cyl(a1 - len, a1, rc), vec![], mat(FILL)));
    }
    // cinta de condução de cobre
    let (b0, b1) = (l - 0.72 * d, l - 0.58 * d);
    regions.push(reg("Cinta de condução", cyl(b0, b1, r), cyl(b0, b1, r - 0.03 * d), mat(CU)));
    Built { profile: prof, body: s.body, regions }
}

// ---------------------------------------------------------------- munições

fn b_9mm() -> Built {
    bullet(9.01, 15.0, 8.0, 0.0, 0.0, 0.5, GILD, PB, vec![])
}
fn b_m855() -> Built {
    // penetrador de aço cônico na frente, chumbo atrás
    bullet(5.70, 22.8, 13.0, 3.5, 2.0, 0.45, GILD, PB, vec![reg("Penetrador de aço", vec![[1.6, 0.0], [5.4, 1.85], [9.6, 1.85]], vec![], mat(AP))])
}
fn b_m80() -> Built {
    bullet(7.82, 28.0, 17.0, 4.5, 3.0, 0.7, GILD, PB, vec![])
}
fn b_m62() -> Built {
    // chumbo na frente, composição traçante num copo aberto na base
    let probe = bullet(7.82, 30.0, 15.5, 3.0, 3.4, 0.7, GILD, PB, vec![]);
    let tr = clip(&shrink(&probe.profile, 0.7), 23.5, 30.0);
    bullet(7.82, 30.0, 15.5, 3.0, 3.4, 0.7, GILD, PB, vec![reg("Traçante", tr, vec![], mat(FILL))])
}
fn b_m61() -> Built {
    // núcleo de aço endurecido com enchimento de chumbo na ponta
    bullet(7.82, 29.5, 15.5, 3.5, 3.3, 0.65, GILD, PB, vec![reg("Núcleo de aço", with(ogive(3.6, 9.5, 3.05), &[[26.0, 3.05], [27.0, 2.6]]), vec![], mat(AP))])
}
fn b_b32() -> Built {
    // camisa bimetálica (aço cladeado), carga incendiária na ponta, núcleo de aço, capa de chumbo
    let probe = bullet(7.92, 32.0, 16.0, 3.0, 3.4, 0.75, MILD, PB, vec![]);
    let inc = clip(&shrink(&probe.profile, 0.75), 0.0, 7.0);
    bullet(7.92, 32.0, 16.0, 3.0, 3.4, 0.75, MILD, PB, vec![
        reg("Incendiária", inc, vec![], mat(FILL)),
        reg("Núcleo de aço", with(ogive(6.0, 10.0, 3.0), &[[29.0, 3.0]]), vec![], mat(AP)),
    ])
}
fn b_50ap() -> Built {
    bullet(12.95, 56.0, 28.0, 9.0, 5.0, 0.85, GILD, PB, vec![reg("Núcleo de aço", with(ogive(7.5, 15.0, 5.45), &[[44.5, 5.45], [48.0, 4.6]]), vec![], mat(AP))])
}
fn b_145() -> Built {
    let probe = bullet(14.88, 62.0, 31.0, 10.0, 5.6, 1.0, MILD, PB, vec![]);
    let inc = clip(&shrink(&probe.profile, 1.0), 0.0, 11.0);
    bullet(14.88, 62.0, 31.0, 10.0, 5.6, 1.0, MILD, PB, vec![
        reg("Incendiária", inc, vec![], mat(FILL)),
        reg("Núcleo de aço", with(ogive(10.0, 16.0, 5.6), &[[54.0, 5.6]]), vec![], mat(AP)),
    ])
}

/// APCBC-HE alemão (PzGr 39/42, 39/43, 43): para-brisa de aço oco, capuz perfurante soldado
/// sobre o nariz ogival do corpo, pequena carga perto da base, espoleta de base, cinta de cobre.
fn apcbc_de(d: f64, l: f64, he_g: f64) -> Built {
    shell(Shell { d, l, ws_len: 1.45, ws_mat: MILD, cap: Some((1.0, 0.22, 1.45, 1.95)), nose_tip: 1.2, nose_len: 1.0, nose_flat: 0.0, he_g, he_r: 0.12, bt: (0.35, 0.44), body: AP })
}
fn s_pzgr39_42() -> Built {
    apcbc_de(75.0, 290.0, 28.0)
}
fn s_pzgr39_43() -> Built {
    apcbc_de(88.0, 308.0, 59.0)
}
fn s_pzgr43_128() -> Built {
    apcbc_de(128.0, 424.0, 400.0)
}
fn s_m61_75() -> Built {
    // APC americano com para-brisa: capuz mais longo, carga de Explosive D (~65 g)
    shell(Shell { d: 75.0, l: 275.0, ws_len: 1.4, ws_mat: MILD, cap: Some((0.95, 0.25, 1.4, 2.0)), nose_tip: 1.2, nose_len: 1.05, nose_flat: 0.0, he_g: 65.0, he_r: 0.13, bt: (0.2, 0.47), body: AP })
}
fn s_br365() -> Built {
    // nariz rombudo (anti-ricochete) sob uma ponta balística oca; sem capuz perfurante
    shell(Shell { d: 85.0, l: 274.0, ws_len: 1.15, ws_mat: MILD, cap: None, nose_tip: 0.75, nose_len: 0.45, nose_flat: 0.30, he_g: 48.0, he_r: 0.10, bt: (0.2, 0.47), body: AP })
}
fn s_br471() -> Built {
    // ponta ogival aguda, sem capuz nem para-brisa; carga de TNT de 156 g
    shell(Shell { d: 122.0, l: 362.0, ws_len: 0.0, ws_mat: MILD, cap: None, nose_tip: 0.0, nose_len: 1.5, nose_flat: 0.0, he_g: 156.0, he_r: 0.10, bt: (0.2, 0.47), body: AP })
}
fn s_m304() -> Built {
    // HVAP: corpo-portador de alumínio de calibre cheio com núcleo de carboneto de tungstênio
    // de 38 mm na frente, sob um para-brisa de aço; o alumínio se desfaz no impacto
    let d = 90.0;
    let r = d / 2.0;
    let l = 360.0;
    let prof = with(ogive(0.0, 1.1 * d, r), &[[l, r]]);
    let ws = 1.1 * d;
    let wsp = clip(&prof, 0.0, ws);
    let core_tip = 0.85 * d;
    let regions = vec![
        reg("Interior do para-brisa", shrink(&wsp, 0.03 * d), vec![], Fill::Void),
        reg("Nariz do corpo", with(ogive(0.95 * d, 0.7 * d, r), &[[l, r]]), vec![], Fill::Body),
        reg("Núcleo de WC", with(ogive(core_tip, 0.45 * d, 19.0), &[[core_tip + 210.0, 19.0]]), vec![], mat(WC)),
        reg("Para-brisa balístico", wsp.clone(), shrink(&wsp, 0.03 * d), mat(MILD)),
        reg("Cinta de condução", cyl(l - 0.7 * d, l - 0.56 * d, r), cyl(l - 0.7 * d, l - 0.56 * d, r - 0.03 * d), mat(CU)),
    ];
    Built { profile: prof, body: AL, regions }
}
fn s_m829a1() -> Built {
    // dardo de urânio empobrecido com ponta balística oca de alumínio (aletas não modeladas)
    let d = 22.0;
    let r = d / 2.0;
    let prof = vec![[0.0, 0.0], [60.0, r], [684.0, r]];
    let wsp = clip(&prof, 0.0, 60.0);
    let regions = vec![
        reg("Interior da ponta", shrink(&wsp, 1.5), vec![], Fill::Void),
        reg("Nariz do dardo", vec![[32.0, 0.0], [54.0, r], [70.0, r]], vec![], Fill::Body),
        reg("Ponta balística", wsp.clone(), shrink(&wsp, 1.5), mat(AL)),
    ];
    Built { profile: prof, body: DU, regions }
}
fn s_mango() -> Built {
    // dois núcleos de liga de tungstênio dentro de uma camisa de aço, ponta balística de aço
    let d = 26.0;
    let r = d / 2.0;
    let prof = vec![[0.0, 0.0], [70.0, r], [570.0, r]];
    let regions = vec![
        reg("Núcleo de W (1)", with(ogive(62.0, 22.0, 12.0), &[[300.0, 12.0]]), vec![], mat(W)),
        reg("Núcleo de W (2)", cyl(305.0, 530.0, 12.0), vec![], mat(W)),
    ];
    Built { profile: prof, body: AP, regions }
}

pub const AMMO: &[Ammo] = &[
    Ammo { name: "9×19 mm Parabellum FMJ", group: "Armas portáteis", note: "Chumbo numa camisa de tombak.",
        caliber_mm: 9.01, length_mm: 15.0, mass_g: 8.0, velocity: 360.0, build: b_9mm, plate_mat: AL, plate_mm: 8.0, scale: 1.0 },
    Ammo { name: "5,56×45 mm M855 (SS109)", group: "Armas portáteis", note: "Penetrador cônico de aço na frente, chumbo atrás, camisa de tombak.",
        caliber_mm: 5.70, length_mm: 22.8, mass_g: 4.0, velocity: 940.0, build: b_m855, plate_mat: MILD, plate_mm: 6.0, scale: 1.5 },
    Ammo { name: "7,62×51 mm NATO M80 (ball)", group: "Armas portáteis", note: "Bala comum: chumbo-antimônio numa camisa de tombak, base cônica.",
        caliber_mm: 7.82, length_mm: 28.0, mass_g: 9.5, velocity: 840.0, build: b_m80, plate_mat: MILD, plate_mm: 8.0, scale: 1.1 },
    Ammo { name: "7,62×51 mm NATO M62 (traçante)", group: "Armas portáteis", note: "Chumbo na frente e composição traçante num copo aberto na base.",
        caliber_mm: 7.82, length_mm: 30.0, mass_g: 9.3, velocity: 855.0, build: b_m62, plate_mat: MILD, plate_mm: 8.0, scale: 1.1 },
    Ammo { name: "7,62×51 mm NATO M61 (AP)", group: "Armas portáteis", note: "Núcleo de aço endurecido, enchimento de chumbo na ponta, camisa de tombak.",
        caliber_mm: 7.82, length_mm: 29.5, mass_g: 9.7, velocity: 840.0, build: b_m61, plate_mat: RHA, plate_mm: 10.0, scale: 1.1 },
    Ammo { name: "7,62×54R B-32 (API)", group: "Armas portáteis", note: "Incendiária na ponta, núcleo de aço com capa de chumbo, camisa bimetálica (aço).",
        caliber_mm: 7.92, length_mm: 32.0, mass_g: 10.0, velocity: 820.0, build: b_b32, plate_mat: RHA, plate_mm: 12.0, scale: 1.1 },
    Ammo { name: ".50 BMG M2 AP", group: "Armas portáteis", note: "Núcleo de aço endurecido (~27 g), chumbo na ponta e na base, camisa de tombak.",
        caliber_mm: 12.95, length_mm: 56.0, mass_g: 46.0, velocity: 890.0, build: b_50ap, plate_mat: RHA, plate_mm: 20.0, scale: 0.7 },
    Ammo { name: "14,5×114 mm B-32 (KPV)", group: "Armas portáteis", note: "Antimaterial soviético: incendiária na ponta, núcleo de aço, camisa bimetálica.",
        caliber_mm: 14.88, length_mm: 62.0, mass_g: 64.0, velocity: 976.0, build: b_145, plate_mat: RHA, plate_mm: 25.0, scale: 0.62 },
    Ammo { name: "75 mm PzGr 39/42 (Panther KwK 42 L/70)", group: "Canhões históricos", note: "APCBC-HE: para-brisa de aço oco, capuz perfurante sobre o nariz do corpo, ~28 g de carga perto da base, cinta de cobre.",
        caliber_mm: 75.0, length_mm: 290.0, mass_g: 7200.0, velocity: 935.0, build: s_pzgr39_42, plate_mat: RHA, plate_mm: 100.0, scale: 0.12 },
    Ammo { name: "88 mm PzGr 39/43 (KwK 43 L/71)", group: "Canhões históricos", note: "APCBC-HE do Tiger II / Nashorn: para-brisa, capuz, ~59 g de carga. Escala reduzida.",
        caliber_mm: 88.0, length_mm: 308.0, mass_g: 10200.0, velocity: 1000.0, build: s_pzgr39_43, plate_mat: RHA, plate_mm: 140.0, scale: 0.10 },
    Ammo { name: "128 mm PzGr 43 (Pak 44 / Jagdtiger)", group: "Canhões históricos", note: "APCBC-HE: o maior perfurante alemão em serviço, contra 300 mm de aço. Escala reduzida.",
        caliber_mm: 128.0, length_mm: 424.0, mass_g: 28300.0, velocity: 920.0, build: s_pzgr43_128, plate_mat: RHA, plate_mm: 300.0, scale: 0.12 },
    Ammo { name: "75 mm M61 APC-T (M3 / Sherman)", group: "Canhões históricos", note: "APCBC-HE americano: para-brisa, capuz, ~65 g de Explosive D, espoleta de base.",
        caliber_mm: 75.0, length_mm: 275.0, mass_g: 6790.0, velocity: 619.0, build: s_m61_75, plate_mat: RHA, plate_mm: 75.0, scale: 0.12 },
    Ammo { name: "85 mm BR-365 APHE (D-5T / ZIS-S-53 soviético)", group: "Canhões históricos", note: "APBC-HE do T-34-85: nariz rombudo sob ponta balística oca, ~48 g de A-IX-2.",
        caliber_mm: 85.0, length_mm: 274.0, mass_g: 9200.0, velocity: 792.0, build: s_br365, plate_mat: RHA, plate_mm: 100.0, scale: 0.1 },
    Ammo { name: "122 mm BR-471 APHE (D-25T / IS-2 soviético)", group: "Canhões históricos", note: "Ponta ogival aguda, sem capuz; ~156 g de TNT.",
        caliber_mm: 122.0, length_mm: 362.0, mass_g: 25000.0, velocity: 800.0, build: s_br471, plate_mat: RHA, plate_mm: 160.0, scale: 0.075 },
    Ammo { name: "90 mm M304 HVAP-T (M26 Pershing)", group: "Canhões históricos", note: "Corpo-portador de alumínio de 90 mm com núcleo de carboneto de tungstênio de 38 mm, sob para-brisa de aço.",
        caliber_mm: 90.0, length_mm: 360.0, mass_g: 7600.0, velocity: 1016.0, build: s_m304, plate_mat: RHA, plate_mm: 200.0, scale: 0.1 },
    Ammo { name: "120 mm M829A1 APFSDS (DU)", group: "Penetradores modernos", note: "Dardo de urânio empobrecido com ponta balística oca de alumínio (sabot e aletas não modelados). Escala reduzida.",
        caliber_mm: 22.0, length_mm: 684.0, mass_g: 4600.0, velocity: 1575.0, build: s_m829a1, plate_mat: RHA, plate_mm: 150.0, scale: 0.3 },
    Ammo { name: "125 mm 3BM42 'Mango' APFSDS (W)", group: "Penetradores modernos", note: "Dois núcleos de liga de tungstênio numa camisa de aço, ponta de aço (sabot e aletas não modelados). Escala reduzida.",
        caliber_mm: 26.0, length_mm: 570.0, mass_g: 4850.0, velocity: 1700.0, build: s_mango, plate_mat: RHA, plate_mm: 150.0, scale: 0.28 },
];

impl Ammo {
    /// Configuração do modelo na escala `scale` (todas as dimensões × scale).
    pub fn config(&self, scale: f64) -> Config {
        let s = scale;
        let b = (self.build)();
        let k = s * 1e-3;
        let mut projectile = Projectile { profile: b.profile, material: LIBRARY[b.body].clone(), core: None, regions: b.regions };
        projectile.scale(k);
        let d = self.caliber_mm * k;
        // alvos de projéteis grandes: 5 calibres de largura (bem menos nós, resultado equivalente)
        let width = ((if self.caliber_mm >= 70.0 { 5.0 } else { 7.0 }) * d).max(0.04);
        Config {
            dx: 0.0015,
            horizon_m: 2.5,
            design: Design {
                objects: vec![],
                projectile,
                layers: vec![Layer {
                    thickness: self.plate_mm * k,
                    gap_before: 0.0,
                    width,
                    height: width,
                    round: false,
                    material: {
                        let mut m = LIBRARY[self.plate_mat].clone();
                        crate::material::adjust_rha(&mut m, self.plate_mm * 1e-3);
                        m
                    },
                }],
            },
            velocity: self.velocity,
            obliquity_deg: 0.0,
            clamp_edges: true,
            time_factor: 1.0,
            n_frames: 100,
            symmetry: true,
            sleep_ratio: 0.05,
            noise: 0.15,
            jitter: 0.05,
            ignore_memory: false,
            rate_gain: 1.0,
            deviatoric_plasticity: false,
        }
    }

    /// Massa do modelo em escala (kg) = massa real × s³.
    pub fn scaled_mass_g(&self, scale: f64) -> f64 {
        self.mass_g * scale.powi(3)
    }
}
