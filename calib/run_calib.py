#!/usr/bin/env python3
"""Roda os casos de calibração esfera × chapa no PenSim e no OpenCourant e compara.

Uso: OC=/caminho/OpenCourant python3 calib/run_calib.py [pasta_de_trabalho]
Saída: calib/ref/esfera_chapa.csv (uma linha por caso).
"""
import csv, os, subprocess, sys, time

OC = os.environ.get("OC", "/home/user/ext/oc/OpenCourant")
WORK = sys.argv[1] if len(sys.argv) > 1 else "/tmp/calib"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "calib", "ref", "esfera_chapa.csv")
ND = os.environ.get("ND", "12")
VEL = {"al": [300, 500, 700, 900, 1100], "rha": [600, 900, 1200, 1500, 1800]}
THICK = [5.0, 10.0]  # 0,5 D e 1 D (esfera Ø10 mm)
ANG = [0, 30, 45, 60]

env = dict(os.environ)
env.update({
    "OPENCOURANT_PATH": OC,
    "LD_LIBRARY_PATH": f"{OC}/extlib/hm_reader/linux64:{OC}/extlib/h3d/lib/linux64",
    "RAD_CFG_PATH": f"{OC}/hm_cfg_files",
    "OMP_STACKSIZE": "400m",
})
os.makedirs(WORK, exist_ok=True)
os.makedirs(os.path.dirname(OUT), exist_ok=True)
subprocess.run(["cargo", "build", "--release", "-q", "-p", "pensim-core", "--example", "calib_case"], cwd=ROOT, check=True)
exe = os.path.join(ROOT, "target", "release", "examples", "calib_case")


def oc_velocity(name):
    """Velocidade do centro de massa da peça 1 (esfera) no fim, do histórico do OpenCourant."""
    subprocess.run([f"{OC}/exec/th_to_csv_linux64_gf", f"{name}T01"], cwd=WORK, env=env, capture_output=True)
    with open(os.path.join(WORK, f"{name}T01.csv")) as f:
        rows = list(csv.reader(f))
    head = [h.strip() for h in rows[0]]
    last = [float(x) for x in rows[-1]]

    def col(tag):
        for i, h in enumerate(head):
            if h.startswith("peca 1") and h.endswith(tag):
                return last[i]
        raise KeyError(tag)

    m = col("MASS")
    return col("XMOM") / m, col("YMOM") / m, col("ZMOM") / m


done = set()
if os.path.exists(OUT):
    with open(OUT) as f:
        done = {r["caso"] for r in csv.DictReader(f)}
new = not os.path.exists(OUT)
with open(OUT, "a", newline="") as fo:
    w = csv.writer(fo)
    if new:
        w.writerow(["caso", "material", "espessura_mm", "angulo_graus", "v_impacto", "nd",
                    "pensim_vx", "pensim_vy", "pensim_vz", "pensim_erodidos", "pensim_erro_energia_pct", "pensim_s",
                    "oc_vx", "oc_vy", "oc_vz", "oc_s"])
    for mat in ["al", "rha"]:
        for t in THICK:
            for a in ANG:
                for v in VEL[mat]:
                    e2 = dict(env, MAT=mat, T=str(t), A=str(a), V=str(v), OUT=WORK, ND=ND, DUR=str(max(40.0, 40.0 * t / 5.0)))
                    r = subprocess.run([exe], env=e2, capture_output=True, text=True)
                    name = next(l.split()[1] for l in r.stdout.splitlines() if l.startswith("NAME"))
                    if name in done:
                        continue
                    res = next(l.split() for l in r.stdout.splitlines() if l.startswith("RESULT"))
                    t0 = time.time()
                    for stage, deck in [("starter", "_0000.rad"), ("engine", "_0001.rad")]:
                        subprocess.run([f"{OC}/exec/{stage}_linux64_gf", "-i", name + deck, "-nt", str(os.cpu_count())], cwd=WORK, env=env, capture_output=True)
                    try:
                        ov = oc_velocity(name)
                    except Exception as ex:  # caso que falhou no OpenCourant
                        print("falhou", name, ex, flush=True)
                        ov = (float("nan"),) * 3
                    w.writerow([name, mat, t, a, v, ND, *res[2:8], *[f"{x:.3f}" for x in ov], f"{time.time() - t0:.1f}"])
                    fo.flush()
                    print(name, "PenSim", res[2], "OpenCourant", f"{ov[0]:.1f}", flush=True)
                    for fn in os.listdir(WORK):
                        if fn.startswith(name) and (fn[len(name):len(name) + 2] in ("A0", "A1") or fn.endswith(".rst")):
                            os.remove(os.path.join(WORK, fn))
