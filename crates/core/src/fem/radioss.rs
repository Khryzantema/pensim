//! Exporta um modelo do núcleo de elementos finitos para o formato do OpenRadioss / OpenCourant
//! (decks `_0000.rad` do starter e `_0001.rad` do engine), com a MESMA malha e os mesmos
//! materiais, para servir de referência (docs/NOVO-NUCLEO.md, fase 3).
//!
//! Unidades do deck: kg, mm, ms (tensão em GPa, velocidade em m/s, energia em J).
//! Formatos conferidos em `hm_cfg_files/config/CFG/radioss2026` e nos testes de QA do OpenRadioss.
use super::material::{Eos, FemMat};
use super::mesh::HexMesh;
use std::fmt::Write;

pub struct DeckInput<'a> {
    pub name: &'a str,
    pub mesh: &'a HexMesh,
    pub mats: &'a [FemMat],
    pub v0: &'a [[f64; 3]],
    pub friction: f64,
    /// tempo final [s]
    pub t_end: f64,
    /// intervalo da saída de histórico [s]
    pub th_dt: f64,
    /// formulação do sólido do Radioss (24 = HEPH, 1 ponto com estabilização física;
    /// 1 = Belytschko 1 ponto com hourglass viscoso; 14 = integração completa)
    pub isolid: i32,
    /// nós com histórico de velocidade (para medir o projétil nó a nó, inclusive o que sobra
    /// de elementos apagados: a massa da peça no histórico do Radioss perde os elementos apagados)
    pub th_nodes: &'a [u32],
}

fn f(v: f64) -> String {
    // campo de 20 colunas, notação científica curta
    format!("{:>20}", format!("{:.9e}", v))
}

/// (starter, engine)
pub fn write(d: &DeckInput) -> (String, String) {
    let m = d.mesh;
    let mut s = String::new();
    let _ = writeln!(s, "#RADIOSS STARTER\n/BEGIN\n{}\n      2026         0", d.name);
    let _ = writeln!(s, "                  kg                  mm                  ms\n                  kg                  mm                  ms");
    // ---- materiais (um por peça)
    for (k, mt) in d.mats.iter().enumerate() {
        let id = k + 1;
        let kb = mt.bulk();
        let nu = ((3.0 * kb - 2.0 * mt.g) / (2.0 * (3.0 * kb + mt.g))).clamp(0.0, 0.499);
        let e = 2.0 * mt.g * (1.0 + nu);
        let j = &mt.jc;
        let a = if j.a > 1e20 { 1e20 } else { j.a };
        let _ = writeln!(s, "/MAT/PLAS_JOHNS/{id}\n{}", mt.name);
        let _ = writeln!(s, "#              RHO_I\n{}{}", f(mt.rho * 1e-9), f(0.0));
        let _ = writeln!(s, "#                  E                  Nu     Iflag\n{}{}         0", f(e * 1e-9), f(nu));
        let _ = writeln!(s, "#                  a                   b                   n           EPS_p_max            SIG_max0\n{}{}{}{}{}", f(a * 1e-9), f(j.b * 1e-9), f(j.n), f(0.0), f(0.0));
        let _ = writeln!(s, "#                  c           EPS_DOT_0       ICC   Fsmooth               F_cut               Chard\n{}{}         0         0{}{}", f(j.c), f(j.eps_dot0 * 1e-3), f(0.0), f(0.0));
        // (sem filtro de taxa: o PenSim também usa a taxa do passo sem filtrar)
        let tm = if mt.t_melt > 1e8 { 0.0 } else { mt.t_melt };
        let _ = writeln!(s, "#                  m              T_melt              rhoC_p                 T_r\n{}{}{}{}", f(j.m), f(tm), f(mt.rho * 1e-9 * mt.cp), f(mt.t_room));
        if let Eos::MieGruneisen { c0, s: s1, gamma0 } = mt.eos {
            let _ = writeln!(s, "/EOS/GRUNEISEN/{id}\n{}", mt.name);
            let _ = writeln!(s, "#                  C                  S1                  S2                  S3\n{}{}{}{}", f(c0), f(s1), f(0.0), f(0.0));
            let _ = writeln!(s, "#                 Y0                   a                  E0\n{}{}{}", f(gamma0), f(0.0), f(0.0));
            let _ = writeln!(s, "#                 P0                 PSH\n{}{}", f(0.0), f(0.0));
        }
        if let Some(dm) = mt.dmg {
            let _ = writeln!(s, "/FAIL/JOHNSON/{id}");
            let _ = writeln!(s, "#                 D1                  D2                  D3                  D4                  D5\n{}{}{}{}{}", f(dm.d[0]), f(dm.d[1]), f(dm.d[2]), f(dm.d[3]), f(dm.d[4]));
            // elemento apagado quando o dano chega a 1 no ponto de integração (Ifail_so = 1)
            let _ = writeln!(s, "#              EPS_0  Ifail_sh  Ifail_so            EPSF_MIN                Dadv               Ixfem\n{}         0         1{}{}                   0", f(j.eps_dot0 * 1e-3), f(0.0), f(0.0));
            let _ = writeln!(s, "#   FAILIP\n         1");
        }
        // ---- propriedade e peça
        let _ = writeln!(s, "/PROP/SOLID/{id}\nsolido");
        let _ = writeln!(s, "#   Isolid    Ismstr      Iale     Icpre  Itetra10     Inpts   Itetra4    Iframe                  dn\n{:>10}         0         0         0         0         0         0         0{}", d.isolid, f(0.0));
        let _ = writeln!(s, "#                 qa                  qb                   h              Lambda                  Mu\n{}{}{}{}{}", f(0.0), f(0.0), f(0.0), f(0.0), f(0.0));
        let _ = writeln!(s, "#         deltaT_min            vdef_min            vdef_max             ASP_max             COL_min\n{}{}{}{}{}", f(0.0), f(0.0), f(0.0), f(0.0), f(0.0));
        let _ = writeln!(s, "#     Ndir sphpartID  Icontrol\n         0         0         0");
        let _ = writeln!(s, "/PART/{id}\npeca {id}\n{:>10}{:>10}         0", id, id);
    }
    // ---- nós
    let _ = writeln!(s, "/NODE");
    for (i, p) in m.x.iter().enumerate() {
        let _ = writeln!(s, "{:>10}{}{}{}", i + 1, f(p[0] * 1e3), f(p[1] * 1e3), f(p[2] * 1e3));
    }
    // ---- hexaedros por peça (ordem de nós igual à do Radioss: 1-4 numa face, 5-8 na oposta)
    for k in 0..d.mats.len() {
        let _ = writeln!(s, "/BRICK/{}", k + 1);
        for (e, h) in m.hex.iter().enumerate() {
            if m.part[e] as usize == k {
                let _ = write!(s, "{:>10}", e + 1);
                for &n in h {
                    let _ = write!(s, "{:>10}", n + 1);
                }
                s.push('\n');
            }
        }
    }
    // ---- grupos de nós: condições de contorno por combinação de eixos fixos
    let mut gid = 100;
    for mask in 1u8..8 {
        let nodes: Vec<usize> = (0..m.n_nodes()).filter(|&i| m.sym[i] == mask).collect();
        if nodes.is_empty() {
            continue;
        }
        gid += 1;
        let _ = writeln!(s, "/GRNOD/NODE/{gid}\nfixos {mask}");
        for c in nodes.chunks(10) {
            for &n in c {
                let _ = write!(s, "{:>10}", n + 1);
            }
            s.push('\n');
        }
        let t = format!("{}{}{}", mask & 1, (mask >> 1) & 1, (mask >> 2) & 1);
        let _ = writeln!(s, "/BCS/{gid}\nfixos {mask}\n#  Tra rot   skew_ID  grnod_ID\n   {t} 000         0{:>10}", gid);
    }
    // ---- velocidade inicial: um grupo por vetor distinto (aqui: parado ou projétil)
    let mut vgroups: Vec<([f64; 3], Vec<usize>)> = Vec::new();
    for (i, v) in d.v0.iter().enumerate() {
        if v.iter().all(|c| *c == 0.0) {
            continue;
        }
        match vgroups.iter_mut().find(|g| g.0 == *v) {
            Some(g) => g.1.push(i),
            None => vgroups.push((*v, vec![i])),
        }
    }
    for (k, (v, nodes)) in vgroups.iter().enumerate() {
        let g = 200 + k + 1;
        let _ = writeln!(s, "/GRNOD/NODE/{g}\nvelocidade {k}");
        for c in nodes.chunks(10) {
            for &n in c {
                let _ = write!(s, "{:>10}", n + 1);
            }
            s.push('\n');
        }
        let _ = writeln!(s, "/INIVEL/TRA/{}\nvelocidade {k}\n#                 Vx                  Vy                  Vz   Gnod_id   Skew_id\n{}{}{}{:>10}         0\n#             tstart   sens_ID\n{}         0", k + 1, f(v[0]), f(v[1]), f(v[2]), g, f(0.0));
    }
    // ---- contato geral (todas as peças contra todas, com erosão)
    let _ = write!(s, "/SURF/PART/ALL/1\ntodas\n");
    for k in 0..d.mats.len() {
        let _ = write!(s, "{:>10}", k + 1);
    }
    s.push('\n');
    let _ = writeln!(s, "/INTER/TYPE25/1\ncontato geral");
    let _ = writeln!(s, "# Surf_ID1  Surf_ID2      Istf      Ithe      Igap   Irem_i2                Idel     Iedge\n         1         0         0         0         0         0                   1         0");
    let _ = writeln!(s, "# grnd_IDS                     Gap_scale          %mesh_size           Gap_max_s           Gap_max_m\n         0          {}{}{}{}", f(0.0), f(0.0), f(0.0), f(0.0));
    let _ = writeln!(s, "#              Stmin               Stmax     Igap0    Ishape          Edge_angle\n{}{}         0         0{}", f(0.0), f(0.0), f(0.0));
    let _ = writeln!(s, "#              Stfac                Fric           Tpressfit              Tstart               Tstop\n{}{}{}{}{}", f(0.0), f(d.friction), f(0.0), f(0.0), f(0.0));
    let _ = writeln!(s, "#      IBC               IVIS2    Inacti               ViscS    Ithick                          Pmax\n       000                   0         0{}         0          {}", f(0.0), f(0.0));
    let _ = writeln!(s, "#    Ifric    Ifiltr               Xfreq             sens_ID                                 fric_ID\n         0         0{}                   0                                       0", f(0.0));
    // ---- histórico: energias e quantidade de movimento de cada peça
    let _ = write!(s, "/TH/PART/1\npecas\n#     var1      var2      var3      var4      var5      var6      var7      var8      var9     var10\nDEF\n#     Obj1      Obj2      Obj3      Obj4      Obj5      Obj6      Obj7      Obj8      Obj9     Obj10\n");
    for k in 0..d.mats.len() {
        let _ = write!(s, "{:>10}", k + 1);
    }
    s.push('\n');
    if !d.th_nodes.is_empty() {
        let _ = writeln!(s, "/TH/NODE/2\nprojetil\n#     var1      var2      var3      var4      var5      var6      var7      var8      var9     var10\nVX        VY        VZ\n#    NODid     Iskew                                           NODname");
        for &n in d.th_nodes {
            let _ = writeln!(s, "{:>10}         0", n + 1);
        }
    }
    let _ = writeln!(s, "/END");
    let ms = |t: f64| t * 1e3;
    let eng = format!(
        "#RADIOSS ENGINE\n/RUN/{}/1/\n{}\n/TFILE/0\n{}\n/ANIM/DT\n{}{}\n/PRINT/-1000\n/END\n",
        d.name,
        f(ms(d.t_end)),
        f(ms(d.th_dt)),
        f(0.0),
        f(ms(d.t_end) / 10.0)
    );
    (s, eng)
}
