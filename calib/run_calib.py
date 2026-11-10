#!/usr/bin/env python3
"""Esfera × chapa: PenSim × OpenCourant no mesmo caso, ao mesmo tempo (mesma malha).

Uso: OC=/caminho/OpenCourant python3 calib/run_calib.py [pasta_de_trabalho]
Variáveis: ND (elementos no diâmetro, padrão 12), DTFRAC (erosão por distorção no OpenCourant,
padrão 0,2), MATS/ANGS para filtrar (ex.: MATS=rha ANGS=0,60), PENSIM_THREADS, OC_THREADS.
Saída: calib/ref/esfera_chapa.csv (acrescenta; casos já presentes são pulados).
"""
import csv, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common

WORK = sys.argv[1] if len(sys.argv) > 1 else "/tmp/calib"
OUT = os.path.join(common.ROOT, "calib", "ref", os.environ.get("CSV", "esfera_chapa.csv"))
ND = os.environ.get("ND", "12")
VEL = {"al": [300, 500, 700, 900, 1100], "rha": [600, 900, 1200, 1500, 1800]}
if os.environ.get("VELS"):
    VEL = {k: [int(x) for x in os.environ["VELS"].split(",")] for k in VEL}
THICK = [5.0, 10.0]  # 0,5 D e 1 D (esfera Ø10 mm)
MATS = os.environ.get("MATS", "al,rha").split(",")
ANGS = [int(a) for a in os.environ.get("ANGS", "0,30,45,60").split(",")]

exe = common.build_example("calib_case")
os.makedirs(os.path.dirname(OUT), exist_ok=True)
done = set()
if os.path.exists(OUT):
    with open(OUT) as f:
        done = {r["caso"] for r in csv.DictReader(f)}
new = not os.path.exists(OUT)
cases = []
for mat in MATS:
    for t in THICK:
        for a in ANGS:
            for v in VEL[mat]:
                cases.append(dict(MAT=mat, T=str(t), A=str(a), V=str(v), ND=ND, DTFRAC=os.environ.get("DTFRAC", "0.2"), DUR=str(max(40.0, 40.0 * t / 5.0))))
with open(OUT, "a", newline="") as fo:
    w = csv.writer(fo)
    if new:
        w.writerow(["caso", "material", "espessura_mm", "angulo_graus", "v_impacto", "nd",
                    "pensim_vx", "pensim_vy", "pensim_vz", "pensim_erodidos", "pensim_erro_energia_pct", "pensim_s",
                    "oc_vx", "oc_vy", "oc_vz", "oc_s"])

    def save(c, name, res, v, secs):
        w.writerow([name, c["MAT"], c["T"], c["A"], c["V"], ND, *res[2:8], *[f"{x:.3f}" for x in v], f"{secs:.1f}"])
        fo.flush()
        print(name, "PenSim", res[2], "OpenCourant", f"{v[0]:.1f}", flush=True)

    common.pipeline(exe, WORK, cases, done, save)
