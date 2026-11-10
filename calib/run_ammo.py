#!/usr/bin/env python3
"""Munições da biblioteca × RHA em ângulo: PenSim × OpenCourant (mesma malha), os dois no
mesmo caso ao mesmo tempo. Compara a velocidade média por nó do projétil inteiro no fim.

Uso: OC=/caminho/OpenCourant python3 calib/run_ammo.py [pasta_de_trabalho]
Variáveis: NODES (no diâmetro, padrão 12), SPIN (mil rpm reais; 0 = sem rotação).
Saída: calib/ref/municao_angulo.csv.
ATENÇÃO: o OpenCourant aborta nesses casos enquanto a ponta da ogiva tiver elementos minúsculos
(raio mínimo em mesh::revolution_x); corrigir isso antes (docs/NOVO-NUCLEO.md, seção 15).
"""
import csv, os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import common

WORK = sys.argv[1] if len(sys.argv) > 1 else "/tmp/calib_ammo"
OUT = os.path.join(common.ROOT, "calib", "ref", os.environ.get("CSV", "municao_angulo.csv"))
NODES = os.environ.get("NODES", "12")
SPIN = os.environ.get("SPIN", "0")
CASES = [(".50 BMG", 20.0), ("M61 APC", 80.0), ("PzGr 39/42", 100.0)]
ANG = [0, 30, 60]

exe = common.build_example("calib_ammo")
done = set()
if os.path.exists(OUT):
    with open(OUT) as f:
        done = {r["caso"] for r in csv.DictReader(f)}
new = not os.path.exists(OUT)
cases = [dict(AMMO=k, A=str(a), PLATE=str(p), NODES=NODES, SPIN=SPIN) for k, p in CASES for a in ANG]
with open(OUT, "a", newline="") as fo:
    w = csv.writer(fo)
    if new:
        w.writerow(["caso", "municao", "chapa_mm", "angulo_graus", "nodes", "spin_mil_rpm",
                    "pensim_vx", "pensim_vy", "pensim_vz", "pensim_erodidos", "pensim_erro_energia_pct", "pensim_s",
                    "oc_vx", "oc_vy", "oc_vz", "oc_s"])

    def save(c, name, res, v, secs):
        w.writerow([name, c["AMMO"], c["PLATE"], c["A"], NODES, SPIN, *res[2:8], *[f"{x:.3f}" for x in v], f"{secs:.1f}"])
        fo.flush()
        print(name, "PenSim", res[2:5], "OpenCourant", [f"{x:.1f}" for x in v], flush=True)

    common.pipeline(exe, WORK, cases, done, save)
