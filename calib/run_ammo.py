#!/usr/bin/env python3
"""Munições da biblioteca × RHA em ângulo: PenSim × OpenCourant (mesma malha).

Uso: OC=/caminho/OpenCourant python3 calib/run_ammo.py [pasta_de_trabalho]
Saída: calib/ref/municao_angulo.csv. Compara a velocidade do centro de massa do projétil
inteiro (todas as peças) no fim.
"""
import csv, os, subprocess, sys, time

OC = os.environ.get("OC", "/home/user/ext/oc/OpenCourant")
WORK = sys.argv[1] if len(sys.argv) > 1 else "/tmp/calib_ammo"
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "calib", "ref", "municao_angulo.csv")
NODES = os.environ.get("NODES", "12")
# (trecho do nome, espessura da chapa em mm reais)
CASES = [(".50 BMG", 20.0), ("M61 APC", 80.0), ("PzGr 39/42", 100.0)]
ANG = [0, 30, 60]

env = dict(os.environ)
env.update({
    "OPENCOURANT_PATH": OC,
    "LD_LIBRARY_PATH": f"{OC}/extlib/hm_reader/linux64:{OC}/extlib/h3d/lib/linux64",
    "RAD_CFG_PATH": f"{OC}/hm_cfg_files",
    "OMP_STACKSIZE": "400m",
})
os.makedirs(WORK, exist_ok=True)
subprocess.run(["cargo", "build", "--release", "-q", "-p", "pensim-core", "--example", "calib_ammo"], cwd=ROOT, check=True)
exe = os.path.join(ROOT, "target", "release", "examples", "calib_ammo")


def oc_nodal_velocity(name):
    """Velocidade média (ponderada pela massa nodal) dos nós do projétil no fim, do histórico
    por nó do OpenCourant. A massa da peça no histórico do Radioss perde os elementos apagados,
    então XMOM/MASS da peça superestima a velocidade quando há erosão."""
    subprocess.run([f"{OC}/exec/th_to_csv_linux64_gf", f"{name}T01"], cwd=WORK, env=env, capture_output=True)
    with open(os.path.join(WORK, f"{name}T01.csv")) as f:
        rows = list(csv.reader(f))
    head = [h.strip() for h in rows[0]]
    last = [float(x) for x in rows[-1]]
    with open(os.path.join(WORK, f"{name}.mass")) as f:
        mass = {int(a): float(b) for a, b in (l.split() for l in f if l.strip())}
    cols = {}
    for i, h in enumerate(head):
        if h.startswith("projetil"):
            cols.setdefault(int(h.split()[1]), []).append(i)
    p, m = [0.0, 0.0, 0.0], 0.0
    for nid, c in cols.items():
        mi = mass[nid]
        for k in range(3):
            p[k] += mi * last[c[k]]
        m += mi
    return [x / m for x in p]


done = set()
if os.path.exists(OUT):
    with open(OUT) as f:
        done = {r["caso"] for r in csv.DictReader(f)}
new = not os.path.exists(OUT)
with open(OUT, "a", newline="") as fo:
    w = csv.writer(fo)
    if new:
        w.writerow(["caso", "municao", "chapa_mm", "angulo_graus", "nodes",
                    "pensim_vx", "pensim_vy", "pensim_vz", "pensim_erodidos", "pensim_erro_energia_pct", "pensim_s",
                    "oc_vx", "oc_vy", "oc_vz", "oc_s"])
    for key, plate in CASES:
        for a in ANG:
            e2 = dict(env, AMMO=key, A=str(a), PLATE=str(plate), OUT=WORK, NODES=NODES)
            r = subprocess.run([exe], env=e2, capture_output=True, text=True)
            lines = r.stdout.splitlines()
            name = next(l.split()[1] for l in lines if l.startswith("NAME"))
            if name in done:
                continue
            nproj = int(next(l.split()[1] for l in lines if l.startswith("NPROJ")))
            res = next(l.split() for l in lines if l.startswith("RESULT"))
            t0 = time.time()
            for stage, deck in [("starter", "_0000.rad"), ("engine", "_0001.rad")]:
                subprocess.run([f"{OC}/exec/{stage}_linux64_gf", "-i", name + deck, "-nt", str(os.cpu_count())], cwd=WORK, env=env, capture_output=True)
            try:
                ov = oc_nodal_velocity(name)
            except Exception as ex:
                print("falhou", name, ex, flush=True)
                ov = [float("nan")] * 3
            w.writerow([name, key, plate, a, NODES, *res[2:8], *[f"{x:.3f}" for x in ov], f"{time.time() - t0:.1f}"])
            fo.flush()
            print(name, "PenSim", res[2:5], "OpenCourant", [f"{x:.1f}" for x in ov], flush=True)
            for fn in os.listdir(WORK):
                if fn.startswith(name) and (fn[len(name):len(name) + 2] in ("A0", "A1") or fn.endswith(".rst")):
                    os.remove(os.path.join(WORK, fn))
