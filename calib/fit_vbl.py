#!/usr/bin/env python3
"""Limite balístico estimado (Recht-Ipson, p=2) por série (material, espessura, ângulo).

Uso: python3 calib/fit_vbl.py [csv ...]   (padrão: calib/ref/baseline_pc.csv)
Ajusta vr² = a²·(v² − vbl²) aos pontos com vr > 60 m/s, para o PenSim e o OpenCourant. Séries
com menos de 2 pontos perfurantes ficam "—". É uma estimativa (2–5 pontos): serve para
comparar solvers e ensaios, não como medida.
"""
import csv, math, os, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
files = sys.argv[1:] or [os.path.join(ROOT, "calib", "ref", "baseline_pc.csv")]


def fit(pts):
    p = [(v * v, vr * vr) for v, vr in pts if vr > 60]
    if len(p) < 2:
        return None
    n = len(p); sx = sum(x for x, _ in p); sy = sum(y for _, y in p)
    sxx = sum(x * x for x, _ in p); sxy = sum(x * y for x, y in p)
    d = n * sxx - sx * sx
    if abs(d) < 1e-9:
        return None
    a2 = (n * sxy - sx * sy) / d
    b = (sy - a2 * sx) / n
    if a2 <= 0 or b >= 0:
        return None
    return math.sqrt(-b / a2), math.sqrt(a2), len(p)


for f in files:
    ser = {}
    for r in csv.DictReader(open(f)):
        k = (r["material"], float(r["espessura_mm"]), int(r["angulo_graus"]))
        ser.setdefault(k, []).append((float(r["v_impacto"]), float(r["pensim_vx"]), float(r["oc_vx"])))
    print(f"# {os.path.basename(f)}")
    print("| série | vbl PenSim | a | vbl OC | a | pontos |")
    print("|---|---|---|---|---|---|")
    fmt = lambda x: f"{x[0]:.0f} | {x[1]:.2f}" if x else "— | —"
    for k, v in sorted(ser.items()):
        fp = fit([(a, b) for a, b, _ in v]); fo = fit([(a, c) for a, _, c in v])
        print(f"| {k[0]} {k[1]:.0f} mm {k[2]}° | {fmt(fp)} | {fmt(fo)} | {fp[2] if fp else 0}/{fo[2] if fo else 0} |")
