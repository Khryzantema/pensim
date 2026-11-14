#!/usr/bin/env bash
# Um caso esfera × chapa nos dois solvers. Variáveis como em calib_case (MAT T A V ND FRIC ISOLID
# DUR PENSIM_HG) e OC (pasta do OpenCourant). Imprime: PenSim vx | OpenCourant vx (por nó).
set -e
OC=${OC:-/home/user/ext/oc/OpenCourant}
W=${W:-/home/user/ext/one}
mkdir -p "$W"
export OPENCOURANT_PATH=$OC LD_LIBRARY_PATH=$OC/extlib/hm_reader/linux64:$OC/extlib/h3d/lib/linux64 RAD_CFG_PATH=$OC/hm_cfg_files OMP_STACKSIZE=400m
ROOT=$(cd "$(dirname "$0")/.." && pwd)
out=$(OUT=$W "$ROOT/target/release/examples/calib_case")
name=$(echo "$out" | awk '/^NAME/{print $2}')
pv=$(echo "$out" | awk '/^RESULT/{print $3}')
er=$(echo "$out" | awk -F'= ' '/^EROSAO/{print $2}')
if [ "${SKIP_OC:-0}" = 1 ]; then echo "$name PenSim $pv"; exit 0; fi
cd "$W"
"$OC/exec/starter_linux64_gf" -i "${name}_0000.rad" -nt "$(nproc)" > /dev/null 2>&1
"$OC/exec/engine_linux64_gf" -i "${name}_0001.rad" -nt "$(nproc)" > /dev/null 2>&1
"$OC/exec/th_to_csv_linux64_gf" "${name}T01" > /dev/null 2>&1
ov=$(python3 -I - "$name" <<'P'
import csv, sys
n = sys.argv[1]
rows = list(csv.reader(open(n + "T01.csv")))
h = [x.strip() for x in rows[0]]; last = [float(x) for x in rows[-1]]
mass = {int(a): float(b) for a, b in (l.split() for l in open(n + ".mass") if l.strip())}
cols = {}
for i, c in enumerate(h):
    if c.startswith("projetil"):
        cols.setdefault(int(c.split()[1]), []).append(i)
p = m = 0.0
for k, c in cols.items():
    p += mass[k] * last[c[0]]; m += mass[k]
print(f"{p / m:.1f}")
P
)
echo "$name PenSim $pv (erosão $er) OpenCourant $ov"
