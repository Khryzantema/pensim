"""Peças comuns dos scripts de calibração (PenSim × OpenCourant).

Ritmo: os dois solvers rodam o MESMO caso ao mesmo tempo (mesma malha e materiais); só depois
de os dois terminarem passa ao próximo. Núcleos: metade para cada um, ou PENSIM_THREADS /
OC_THREADS.
"""
import csv, os, subprocess, threading, time

OC = os.environ.get("OC", "/home/user/ext/oc/OpenCourant")
ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
NCPU = os.cpu_count() or 4
PENSIM_THREADS = int(os.environ.get("PENSIM_THREADS", max(1, NCPU // 2)))
OC_THREADS = int(os.environ.get("OC_THREADS", max(1, NCPU - PENSIM_THREADS)))

ENV = dict(os.environ)
ENV.update({
    "OPENCOURANT_PATH": OC,
    "LD_LIBRARY_PATH": f"{OC}/extlib/hm_reader/linux64:{OC}/extlib/h3d/lib/linux64",
    "RAD_CFG_PATH": f"{OC}/hm_cfg_files",
    "OMP_STACKSIZE": "400m",
    "RAYON_NUM_THREADS": str(PENSIM_THREADS),
})


def build_example(name):
    subprocess.run(["cargo", "build", "--release", "-q", "-p", "pensim-core", "--example", name], cwd=ROOT, check=True)
    return os.path.join(ROOT, "target", "release", "examples", name)


def run_oc(work, name):
    """Starter + engine do OpenCourant; devolve (velocidade média por nó do projétil, segundos)."""
    t0 = time.time()
    for stage, deck in [("starter", "_0000.rad"), ("engine", "_0001.rad")]:
        subprocess.run([f"{OC}/exec/{stage}_linux64_gf", "-i", name + deck, "-nt", str(OC_THREADS)], cwd=work, env=ENV, capture_output=True)
    try:
        v = oc_nodal_velocity(work, name)
    except Exception as ex:
        print("falhou no OpenCourant:", name, ex, flush=True)
        v = [float("nan")] * 3
    out = os.path.join(work, f"{name}_0001.out")
    if os.path.exists(out) and "TOO HIGH" in open(out, errors="replace").read():
        print("AVISO: OpenCourant abortou no contato em", name, flush=True)
    for fn in os.listdir(work):
        if fn.startswith(name) and (fn[len(name):len(name) + 2] in ("A0", "A1") or fn.endswith(".rst")):
            os.remove(os.path.join(work, fn))
    return v, time.time() - t0


def oc_nodal_velocity(work, name):
    """Velocidade média (ponderada pela massa nodal) dos nós do projétil no fim. A massa da peça
    no histórico do Radioss perde os elementos apagados; por isso a média é por nó."""
    subprocess.run([f"{OC}/exec/th_to_csv_linux64_gf", f"{name}T01"], cwd=work, env=ENV, capture_output=True)
    with open(os.path.join(work, f"{name}T01.csv")) as f:
        rows = list(csv.reader(f))
    head = [h.strip() for h in rows[0]]
    last = [float(x) for x in rows[-1]]
    with open(os.path.join(work, f"{name}.mass")) as f:
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


def pensim(exe, env_extra):
    """Roda um caso no PenSim (e escreve o deck). Devolve (nome, campos do RESULT)."""
    r = subprocess.run([exe], env=dict(ENV, **env_extra), capture_output=True, text=True)
    lines = r.stdout.splitlines()
    name = next(l.split()[1] for l in lines if l.startswith("NAME"))
    res = next((l.split() for l in lines if l.startswith("RESULT")), None)
    if res is None:
        print("falhou no PenSim:", name, r.stderr[-500:], flush=True)
        res = ["RESULT", name] + ["nan"] * 6
    return name, res


def case_name(exe, env_extra):
    """Só o nome do caso (sem simular), para pular os que já estão no CSV."""
    r = subprocess.run([exe], env=dict(ENV, RUN="0", **env_extra), capture_output=True, text=True)
    return next(l.split()[1] for l in r.stdout.splitlines() if l.startswith("NAME"))


def pipeline(exe, work, cases, done, on_result):
    """Para cada caso: escreve o deck, roda PenSim e OpenCourant AO MESMO TEMPO no mesmo caso,
    espera os dois e passa ao próximo. cases: lista de dicts com as variáveis de ambiente de cada
    caso. on_result(caso, nome, campos_pensim, v_oc, segundos_oc) é chamado na ordem."""
    os.makedirs(work, exist_ok=True)
    for c in cases:
        e = dict(c, OUT=work)
        name = case_name(exe, e)  # também escreve o deck
        if name in done:
            continue
        box = {}
        th = threading.Thread(target=lambda: box.update(oc=run_oc(work, name)))
        th.start()
        _, res = pensim(exe, dict(e, NODECK="1"))
        th.join()
        v, secs = box["oc"]
        on_result(c, name, res, v, secs)
