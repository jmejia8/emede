#!/usr/bin/env python3
"""Benchmark harness: installed emede v0.2.17 vs the deps/semver-updates branch.

Usage:
  run_bench.py --corpus DIR --out results.csv --reps 30 \
      --plain installed=/path/emede --plain branch=/path/emede \
      --bench installed=/path/emede --bench branch=/path/emede \
      [--measures export,print,open] [--docs a,b] [--settings settings.json]

Measures (one row per run, long format):
  export  wall time of `emede --export DOC -o OUT.html` (unmodified binary;
          Rust pipeline + page assembly, headless, no WebView)
  print   wall time of `emede --print DOC -o OUT.pdf` (unmodified binary;
          full WebView: Rust render, MathJax, Mermaid, PDF write)
  open    instrumented build, normal reader path (`emede DOC`) with the window
          parked off-screen; per-phase marks from performance.now() plus the
          Rust render time, peak RSS, and wall time spawn -> report.

Every run starts from a fresh, isolated XDG config/data/cache/state tree holding
only a copy of settings.json (update check forced off), so no run sees view
state, window state, recents or WebKit caches from a previous one. Arms are
interleaved and their order shuffled within each (rep, doc, measure) cell;
rep 0 is a warm-up and is flagged `warmup=1`.
"""
import argparse
import csv
import json
import os
import random
import shutil
import subprocess
import sys
import time
from pathlib import Path

SCRATCH = Path(os.environ.get("BENCH_SCRATCH", "/tmp/emede-bench"))
TIMEOUT = 180


def fresh_env(settings):
    root = SCRATCH / "xdg"
    shutil.rmtree(root, ignore_errors=True)
    env = dict(os.environ)
    for k in ("CONFIG", "DATA", "CACHE", "STATE"):
        d = root / k.lower()
        d.mkdir(parents=True)
        env[f"XDG_{k}_HOME"] = str(d)
    (root / "config" / "emede").mkdir()
    (root / "config" / "emede" / "settings.json").write_text(json.dumps(settings))
    env.pop("EMEDE_BENCH", None)
    return env


def run_wait4(cmd, env, timeout=TIMEOUT, want_bench=False):
    """Spawn, return (wall_s, exit, maxrss_kb, bench_json|None).

    stdout goes to a file (read after exit). For the instrumented build the
    time to report is taken from the report's own wall-clock stamp minus the
    spawn stamp, so process teardown is excluded from it.
    """
    log = SCRATCH / "stdout.txt"
    epoch0 = time.time() * 1000.0
    t0 = time.perf_counter()
    with open(log, "w") as fh:
        p = subprocess.Popen(cmd, env=env, stdout=fh, stderr=subprocess.DEVNULL)
    deadline = t0 + timeout
    while True:
        pid, status, ru = os.wait4(p.pid, os.WNOHANG)
        if pid != 0:
            wall = time.perf_counter() - t0
            rc = os.waitstatus_to_exitcode(status)
            p.returncode = rc
            break
        if time.perf_counter() > deadline:
            p.kill()
            p.wait()
            subprocess.run(["pkill", "-f", str(SCRATCH / "xdg")], check=False)
            return float("nan"), -9, 0, None
        time.sleep(0.001)
    bench = None
    if want_bench:
        for line in log.read_text().splitlines():
            if line.startswith("EMEDE_BENCH "):
                bench = json.loads(line[len("EMEDE_BENCH "):])
                bench["_spawn_epoch_ms"] = epoch0
                bench["_wall_exit_s"] = wall
    return wall, rc, ru.ru_maxrss, bench


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--corpus", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--reps", type=int, default=30)
    ap.add_argument("--plain", action="append", default=[])
    ap.add_argument("--bench", action="append", default=[])
    ap.add_argument("--measures", default="export,print,open")
    ap.add_argument("--docs", default="")
    ap.add_argument("--settings", default=str(Path.home() / ".config/emede/settings.json"))
    ap.add_argument("--seed", type=int, default=20261103)
    a = ap.parse_args()

    plain = dict(x.split("=", 1) for x in a.plain)
    bench = dict(x.split("=", 1) for x in a.bench)
    measures = a.measures.split(",")
    settings = json.loads(Path(a.settings).read_text())
    settings["update_check"] = False
    docs = sorted(Path(a.corpus).glob("*.md"))
    if a.docs:
        keep = set(a.docs.split(","))
        docs = [d for d in docs if d.stem in keep]
    rng = random.Random(a.seed)
    outdir = SCRATCH / "outputs"
    outdir.mkdir(parents=True, exist_ok=True)

    new = not Path(a.out).exists()
    f = open(a.out, "a", newline="")
    w = csv.writer(f)
    if new:
        w.writerow(["measure", "arm", "doc", "rep", "warmup", "wall_s", "exit", "maxrss_kb", "extra"])

    total = (a.reps + 1) * len(docs) * len(measures) * 2
    done = 0
    t_start = time.time()
    for rep in range(a.reps + 1):
        order = docs[:]
        rng.shuffle(order)
        for doc in order:
            for m in measures:
                arms = sorted(bench if m == "open" else plain)
                rng.shuffle(arms)
                for arm in arms:
                    env = fresh_env(settings)
                    if m == "export":
                        cmd = [plain[arm], "--export", str(doc), "-o", str(outdir / f"{arm}-{doc.stem}.html")]
                        wall, rc, rss, b = run_wait4(cmd, env)
                    elif m == "print":
                        cmd = [plain[arm], "--print", str(doc), "-o", str(outdir / f"{arm}-{doc.stem}.pdf")]
                        wall, rc, rss, b = run_wait4(cmd, env)
                    else:
                        env["EMEDE_BENCH"] = "1"
                        wall, rc, rss, b = run_wait4([bench[arm], str(doc)], env, want_bench=True)
                        if b is None:
                            rc = rc if rc else -1
                    w.writerow([m, arm, doc.stem, rep, int(rep == 0), f"{wall:.6f}", rc, rss,
                                json.dumps(b) if b else ""])
                    f.flush()
                    done += 1
                    if rc != 0:
                        print(f"FAIL {m} {arm} {doc.stem} rep={rep} rc={rc}", file=sys.stderr, flush=True)
        el = time.time() - t_start
        print(f"rep {rep}/{a.reps} done ({done}/{total} runs, {el/60:.1f} min, "
              f"eta {el/done*(total-done)/60:.1f} min)", flush=True)
    f.close()


if __name__ == "__main__":
    main()
