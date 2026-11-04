#!/usr/bin/env bash
# Reproduce the benchmark. Usage: run_all.sh <scratch-dir>
# <scratch-dir> must hold corpus/ (gen_corpus.py output) and the two
# instrumented builds target-{installed,branch}/release/emede (see report).
set -euo pipefail
S=$1
HERE=$(cd "$(dirname "$0")" && pwd)
R=$(dirname "$HERE")
INSTALLED=${INSTALLED:-$HOME/.local/bin/emede}
BRANCH=${BRANCH:-$(git -C "$R" rev-parse --show-toplevel)/src-tauri/target/release/emede}
DOCS=prose-M,math-M,mermaid-M,mixed-M
ARGS=(--corpus "$S/corpus" --docs "$DOCS" --reps 30
      --plain "installed=$INSTALLED" --plain "branch=$BRANCH"
      --bench "installed=$S/target-installed/release/emede"
      --bench "branch=$S/target-branch/release/emede")
mkdir -p "$R/raw"
export BENCH_SCRATCH=$S/run
# Phase 1: reader path + export. Phase 2: --print, run alone afterwards.
python3 "$HERE/run_bench.py" "${ARGS[@]}" --measures open,export --out "$R/raw/open_export.csv"
python3 "$HERE/run_bench.py" "${ARGS[@]}" --measures print --out "$R/raw/print.csv"
