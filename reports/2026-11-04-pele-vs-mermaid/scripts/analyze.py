# /// script
# requires-python = ">=3.11"
# dependencies = ["pandas", "numpy", "matplotlib"]
# ///
"""Tables and figures for the Pele vs Mermaid report.

Usage: uv run --script scripts/analyze.py <report-dir>

Reads raw/correctness.json and raw/timing.csv (written by scripts/bench.mjs),
writes tables/*.csv|md and figures/render_time.png|pdf. The significance
table is produced separately by the experiment-report stats_table.py (see
report.md, "Reproduce").

Label completeness: text is reduced to lowercase words (regex \w+, Unicode).
The author's words are the distinct words that appear both in the diagram
source and in Mermaid's rendered text (this drops
generated text such as axis ticks and percentages). Recall is the share of
those distinct tokens that also appear in Pele's rendered text.
"""
import json
import re
import sys
from pathlib import Path

import numpy as np
import pandas as pd

root = Path(sys.argv[1] if len(sys.argv) > 1 else ".")
sys.path.insert(0, str(root / "scripts"))
import plot_style as ps  # noqa: E402

tables = root / "tables"
tables.mkdir(exist_ok=True)

# ---- correctness / completeness / size ---------------------------------------
info = json.loads((root / "raw" / "correctness.json").read_text())
rows = []
for r in info:
    words = lambda toks: set(re.findall(r"\w+", " ".join(toks).lower()))
    src_tokens = words([(root / "corpus" / f"{r['id']}.mmd").read_text()])
    m, p = r["mermaid"], r["pele"]
    row = {"diagram": r["id"], "type": r["type"], "mermaid_ok": m["ok"], "pele_ok": p["ok"]}
    if m["ok"] and p["ok"]:
        author = words(m["tokens"]) & src_tokens
        found = author & words(p["tokens"])
        row |= {
            "author_tokens": len(author),
            "pele_recall": len(found) / len(author) if author else np.nan,
            "missing_in_pele": " ".join(sorted(author - found)),
            "mermaid_svg_kb": m["bytes"] / 1024,
            "pele_svg_kb": p["bytes"] / 1024,
            "mermaid_wxh": f"{m['w']}×{m['h']}",
            "pele_wxh": f"{p['w']}×{p['h']}",
            "height_ratio": p["h"] / m["h"],
        }
    rows.append(row)
cor = pd.DataFrame(rows)
cor.to_csv(tables / "correctness.csv", index=False)

md = ["| Diagram | Parses (M / P) | Author tokens | Pele label recall ↑ | SVG KiB, Mermaid | SVG KiB, Pele | Size at 760 px, Mermaid | Size at 760 px, Pele |",
      "|---|---|---|---|---|---|---|---|"]
for _, r in cor.iterrows():
    ok = f"{'✓' if r.mermaid_ok else '✗'} / {'✓' if r.pele_ok else '✗'}"
    md.append(f"| {r.diagram} | {ok} | {r.author_tokens} | {r.pele_recall:.0%} | {r.mermaid_svg_kb:.1f} | "
              f"**{r.pele_svg_kb:.1f}** | {r.mermaid_wxh} | {r.pele_wxh} |")
md.append(f"| **Total / mean** | {int(cor.mermaid_ok.sum())} / {int(cor.pele_ok.sum())} | {int(cor.author_tokens.sum())} | "
          f"{cor.pele_recall.mean():.0%} | {cor.mermaid_svg_kb.sum():.1f} | **{cor.pele_svg_kb.sum():.1f}** | | |")
(tables / "correctness.md").write_text("\n".join(md) + "\n")

# ---- timing ------------------------------------------------------------------
t = pd.read_csv(root / "raw" / "timing.csv")
g = t.groupby(["diagram", "library"]).time_ms
summ = g.agg(["mean", "std", "median"]).unstack("library")
speed = (summ[("median", "Mermaid")] / summ[("median", "Pele")]).rename("speedup_median")
out = pd.concat([summ, speed], axis=1)
out.columns = ["_".join(c) if isinstance(c, tuple) else c for c in out.columns]
out.to_csv(tables / "timing_summary.csv")

tot = t.groupby(["library", "run"]).time_ms.sum().groupby("library").agg(["mean", "std"])
speed_md = ["| Diagram | Median speed-up (Mermaid / Pele) ↑ |", "|---|---|"]
speed_md += [f"| {d} | {s:.0f}× |" for d, s in speed.items()]
speed_md.append(f"| **All 14, geometric mean** | **{np.exp(np.log(speed).mean()):.0f}×** |")
(tables / "speedup.md").write_text("\n".join(speed_md) + "\n")
(tables / "timing_total.md").write_text(
    "| Library | Whole corpus, ms per pass ↓ (mean ± std) |\n|---|---|\n"
    + "".join(f"| {lib} | {r['mean']:.2f} ± {r['std']:.2f} |\n" for lib, r in tot.iterrows()))

# ---- figure: render time per diagram, log scale ------------------------------
ps.apply()
diagrams = sorted(t.diagram.unique(), reverse=True)  # first diagram on top
fig, ax = ps.figure(width=7.0, height=6.2)
pos = np.arange(len(diagrams))
styles = {"Mermaid": dict(off=0.18, fc="0.85"), "Pele": dict(off=-0.18, fc="0.35")}
handles = []
for lib, st in styles.items():
    data = [t[(t.diagram == d) & (t.library == lib)].time_ms.values for d in diagrams]
    bp = ax.boxplot(data, positions=pos + st["off"], widths=0.3, orientation="horizontal",
                    patch_artist=True, showfliers=True,
                    flierprops=dict(marker=".", markersize=3, markerfacecolor="0.3", markeredgecolor="0.3"),
                    medianprops=dict(color="black"))
    for b in bp["boxes"]:
        b.set_facecolor(st["fc"])
    handles.append((bp["boxes"][0], lib))
ax.set_yticks(pos, diagrams)
ax.set(xscale="log", xlabel="Render + insert + layout time per diagram (ms, log scale)",
       title="Render time per diagram (30 runs × 10 renders)")
ax.legend([h for h, _ in handles], [l for _, l in handles], frameon=False,
          loc="upper center", bbox_to_anchor=(0.5, -0.1), ncol=2)
ps.save(fig, str(root / "figures" / "render_time"))
print(out.round(3).to_string())
print(tot)
print(cor[["diagram", "pele_recall", "missing_in_pele", "height_ratio"]].to_string())
