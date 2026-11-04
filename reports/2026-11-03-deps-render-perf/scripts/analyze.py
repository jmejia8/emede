# /// script
# requires-python = ">=3.11"
# dependencies = ["pandas", "numpy", "scipy", "matplotlib"]
# ///
"""Tables and figures for the deps-render-perf report.

Usage: uv run --script analyze.py <report-dir>

Reads raw/open_export.csv and raw/print.csv (one row per run, warm-up rows
flagged and dropped), writes tables/*.csv|md and figures/*.png|pdf.

Statistics: per (metric, document), two-sided Mann-Whitney U on the 30 runs of
each arm (independent runs, not paired), Holm correction over every test in
the report, alpha = 0.05. Effect size: Vargha-Delaney A12 = P(branch > installed)
+ 0.5 P(tie); for times, A12 < 0.5 means the branch tends to be faster.
Relative change: median(branch) / median(installed) - 1, with a 95 % percentile
bootstrap CI (5000 resamples, seed 1).
"""
import json
import sys
from pathlib import Path

import numpy as np
import pandas as pd
from scipy.stats import mannwhitneyu

R = Path(sys.argv[1])
sys.path.insert(0, str(R / "scripts"))
import plot_style as ps  # noqa: E402

ALPHA = 0.05
DOCS = ["prose-M", "math-M", "mermaid-M", "mixed-M"]
ARMS = ["installed", "branch"]
LABEL = {"installed": "Installed v0.2.17", "branch": "Branch 8fa4084"}


def load():
    oe = pd.read_csv(R / "raw/open_export.csv")
    pr = pd.read_csv(R / "raw/print.csv") if (R / "raw/print.csv").exists() else pd.DataFrame()
    df = pd.concat([oe, pr], ignore_index=True)
    fails = df[(df.exit != 0) & (df.warmup == 0)]
    df = df[(df.warmup == 0) & (df.exit == 0)].copy()
    rows = []
    for _, r in df.iterrows():
        base = dict(arm=r.arm, doc=r.doc, rep=r.rep)
        if r.measure == "export":
            rows.append({**base, "metric": "export_wall_ms", "value": r.wall_s * 1000})
            rows.append({**base, "metric": "export_rss_mb", "value": r.maxrss_kb / 1024})
        elif r.measure == "print":
            rows.append({**base, "metric": "print_wall_ms", "value": r.wall_s * 1000})
        else:
            b = json.loads(r.extra)
            j = b["js"]
            m = {
                "ready_ms": j["painted"],
                "launch_ready_ms": b["report_epoch_ms"] - b["_spawn_epoch_ms"],
                "boot_ms": j["boot"],
                "pre_invoke_ms": j["invoke_start"] - j["boot"],
                "rust_render_ms": b["rust_render_ms"],
                "ipc_ms": (j["invoke_end"] - j["invoke_start"]) - b["rust_render_ms"],
                "dom_apply_ms": j["dyn_start"] - j["invoke_end"],
                "mathjax_ms": j["math_end"] - j["dyn_start"],
                "mermaid_ms": j["mermaid_end"] - j["dyn_start"],
                "dynamic_ms": j["dyn_settled"] - j["dyn_start"],
                "post_layout_ms": j["painted"] - j["dyn_settled"],
                "rss_main_mb": b["rss_main_kb"] / 1024,
                "rss_web_mb": b["rss_children_kb"] / 1024,
                "math_nodes": j["math_nodes"],
                "mermaid_svgs": j["mermaid_svgs"],
            }
            rows += [{**base, "metric": k, "value": v} for k, v in m.items()]
    return pd.DataFrame(rows), fails


def a12(x, y):
    """P(X > Y) + 0.5 P(X = Y), X = branch, Y = installed."""
    x, y = np.asarray(x), np.asarray(y)
    gt = (x[:, None] > y[None, :]).sum()
    eq = (x[:, None] == y[None, :]).sum()
    return (gt + 0.5 * eq) / (len(x) * len(y))


def boot_ci(x, y, rng, n=5000):
    x, y = np.asarray(x), np.asarray(y)
    bx = np.median(rng.choice(x, (n, len(x))), axis=1)
    by = np.median(rng.choice(y, (n, len(y))), axis=1)
    r = bx / by - 1
    return np.percentile(r, [2.5, 97.5])


def holm(p):
    p = np.asarray(p)
    order = np.argsort(p)
    m = len(p)
    adj = np.empty(m)
    run = 0.0
    for k, i in enumerate(order):
        run = max(run, (m - k) * p[i])
        adj[i] = min(1.0, run)
    return adj


TESTED = [
    "ready_ms", "launch_ready_ms", "export_wall_ms", "print_wall_ms",
    "boot_ms", "pre_invoke_ms", "rust_render_ms", "ipc_ms", "dom_apply_ms", "mathjax_ms", "mermaid_ms",
    "post_layout_ms", "rss_main_mb", "rss_web_mb", "export_rss_mb",
]


CENSORED = set()


def hang_table():
    """Timeouts per (doc, arm) for --print; Fisher exact test per document."""
    from scipy.stats import fisher_exact
    pr = pd.read_csv(R / "raw/print.csv")
    pr = pr[pr.warmup == 0]
    rows = []
    for doc in DOCS:
        d = pr[pr.doc == doc]
        h = {a: int((d[d.arm == a].exit != 0).sum()) for a in ARMS}
        n = {a: int((d.arm == a).sum()) for a in ARMS}
        if h["installed"] + h["branch"]:
            CENSORED.add(("print_wall_ms", doc))
        p = fisher_exact([[h["installed"], n["installed"] - h["installed"]],
                          [h["branch"], n["branch"] - h["branch"]]]).pvalue
        rows.append(dict(doc=doc, hang_installed=h["installed"], n_installed=n["installed"],
                         hang_branch=h["branch"], n_branch=n["branch"], fisher_p=p))
    return pd.DataFrame(rows)


def compare(long):
    rng = np.random.default_rng(1)
    out = []
    for metric in TESTED:
        for doc in DOCS:
            sub = long[(long.metric == metric) & (long.doc == doc)]
            x = sub[sub.arm == "branch"].value.values
            y = sub[sub.arm == "installed"].value.values
            if len(x) == 0 or len(y) == 0:
                continue
            # censored by hangs (timeouts): reported as a hang rate instead
            if (metric, doc) in CENSORED:
                continue
            # a phase that does not occur for this document (all ~0) is not tested
            if metric in ("mathjax_ms", "mermaid_ms") and max(np.median(x), np.median(y)) < 20:
                continue
            p = mannwhitneyu(x, y, alternative="two-sided").pvalue
            lo, hi = boot_ci(x, y, rng)
            out.append(dict(
                metric=metric, doc=doc, n_inst=len(y), n_branch=len(x),
                inst_mean=y.mean(), inst_std=y.std(ddof=1), inst_median=np.median(y),
                branch_mean=x.mean(), branch_std=x.std(ddof=1), branch_median=np.median(x),
                delta_median_pct=100 * (np.median(x) / np.median(y) - 1),
                ci_lo_pct=100 * lo, ci_hi_pct=100 * hi, p=p, a12=a12(x, y),
            ))
    res = pd.DataFrame(out)
    res["p_holm"] = holm(res.p.values)
    res["marker"] = np.where(
        res.p_holm >= ALPHA, "≈", np.where(res.branch_median < res.inst_median, "+", "−")
    )
    return res


def fmt(v):
    if abs(v) >= 1000:
        return f"{v:,.0f}"
    if abs(v) >= 100:
        return f"{v:.0f}"
    if abs(v) >= 10:
        return f"{v:.1f}"
    return f"{v:.2f}"


def fmt_p(p):
    return "<1e-4" if p < 1e-4 else f"{p:.3g}"


def md_table(res, metrics, names):
    lines = [
        "| Metric | Document | Installed v0.2.17 | Branch 8fa4084 | Δ median (95 % CI) | p (Holm) | Â₁₂ |",
        "|---|---|---|---|---|---|---|",
    ]
    for metric in metrics:
        for _, r in res[res.metric == metric].iterrows():
            i = f"{fmt(r.inst_mean)} ± {fmt(r.inst_std)}"
            b = f"{fmt(r.branch_mean)} ± {fmt(r.branch_std)}"
            if r.marker != "≈":
                if r.branch_median < r.inst_median:
                    b = f"**{b}**"
                else:
                    i = f"**{i}**"
            b += f" {r.marker}"
            lines.append(
                f"| {names[metric]} | {r.doc} | {i} | {b} | "
                f"{r.delta_median_pct:+.1f} % ({r.ci_lo_pct:+.1f}, {r.ci_hi_pct:+.1f}) | "
                f"{fmt_p(r.p_holm)} | {r.a12:.2f} |"
            )
    return "\n".join(lines)


NAMES = {
    "ready_ms": "Navigation → rendered (ms) ↓",
    "launch_ready_ms": "Launch → rendered (ms) ↓",
    "export_wall_ms": "`--export` wall (ms) ↓",
    "print_wall_ms": "`--print` wall (ms) ↓",
    "boot_ms": "Navigation → boot() (ms) ↓",
    "pre_invoke_ms": "boot() → render invoke (ms) ↓",
    "rust_render_ms": "Rust render (ms) ↓",
    "ipc_ms": "IPC overhead (ms) ↓",
    "dom_apply_ms": "DOM apply (ms) ↓",
    "mathjax_ms": "MathJax done (ms) ↓",
    "mermaid_ms": "Mermaid done (ms) ↓",
    "post_layout_ms": "Post-typeset layout + paint (ms) ↓",
    "rss_main_mb": "Peak RSS, main (MiB) ↓",
    "rss_web_mb": "Peak RSS, WebKit children (MiB) ↓",
    "export_rss_mb": "Peak RSS, `--export` (MiB) ↓",
}


def figures(long):
    ps.apply()
    # Fig 1: time to rendered, horizontal boxes per doc x arm
    fig, ax = ps.figure()
    data, labels, pos = [], [], []
    k = 0
    for doc in DOCS[::-1]:
        for arm in ARMS[::-1]:
            v = long[(long.metric == "ready_ms") & (long.doc == doc) & (long.arm == arm)].value.values
            data.append(v)
            labels.append(f"{doc}, {'branch' if arm == 'branch' else 'v0.2.17'}")
            pos.append(k)
            k += 1
        k += 0.6
    bp = ax.boxplot(data, positions=pos, orientation="horizontal", widths=0.7, patch_artist=True,
                    flierprops=dict(marker=".", markersize=3))
    for i, patch in enumerate(bp["boxes"]):
        patch.set_facecolor("#ffffff" if i % 2 == 0 else "#bbbbbb")
    for med in bp["medians"]:
        med.set_color("black")
    ax.set_yticks(pos, labels)
    ax.set(title="Time to fully rendered document", xlabel="Navigation start → rendered and painted (ms)")
    ps.save(fig, str(R / "figures/ready_boxplot"))

    # Fig 2: median phase breakdown, stacked horizontal bars
    phases = [("boot_ms", "Page load"), ("pre_invoke_ms", "UI wiring"),
              ("rust_render_ms", "Rust render"),
              ("ipc_ms", "IPC"), ("dom_apply_ms", "DOM apply"),
              ("dynamic_ms", "MathJax ∥ Mermaid"), ("post_layout_ms", "Layout + paint")]
    fig, ax = ps.figure(width=6.0, height=3.8)
    shades = ["#000000", "#333333", "#555555", "#777777", "#999999", "#c4c4c4", "#eeeeee"]
    hatches = ["", "", "", "", "//", "", ".."]
    ylab, y = [], 0
    ys = []
    for doc in DOCS[::-1]:
        for arm in ARMS[::-1]:
            left = 0.0
            # boot is measured from navigation start; invoke starts a little later
            for (m, name), c, h in zip(phases, shades, hatches):
                v = long[(long.metric == m) & (long.doc == doc) & (long.arm == arm)].value.median()
                ax.barh(y, v, left=left, color=c, hatch=h, edgecolor="black", linewidth=0.4,
                        label=name if (doc == DOCS[-1] and arm == ARMS[-1]) else None)
                left += v
            ylab.append(f"{doc}, {'branch' if arm == 'branch' else 'v0.2.17'}")
            ys.append(y)
            y += 1
        y += 0.6
    ax.set_yticks(ys, ylab)
    ax.set(title="Median phase breakdown", xlabel="Time since navigation start (ms)")
    h, lab = ax.get_legend_handles_labels()
    fig.legend(h, lab, loc="outside lower center", ncol=4, frameon=False)
    ps.save(fig, str(R / "figures/phase_breakdown"))

    # Fig 3: headless measures (export, print)
    fig, axes = ps.figure(ncols=2, width=7.0, height=3.2)
    for ax, metric, title in zip(axes, ["export_wall_ms", "print_wall_ms"],
                                 ["`--export` (no WebView)", "`--print` (WebView → PDF)"]):
        data, labels, pos = [], [], []
        k = 0
        for doc in DOCS[::-1]:
            for arm in ARMS[::-1]:
                v = long[(long.metric == metric) & (long.doc == doc) & (long.arm == arm)].value.values
                if len(v) == 0:
                    continue
                data.append(v)
                labels.append(f"{doc}, {'branch' if arm == 'branch' else 'v0.2.17'}")
                pos.append(k)
                k += 1
            k += 0.6
        if not data:
            continue
        bp = ax.boxplot(data, positions=pos, orientation="horizontal", widths=0.7, patch_artist=True,
                        flierprops=dict(marker=".", markersize=3))
        for i, patch in enumerate(bp["boxes"]):
            patch.set_facecolor("#ffffff" if i % 2 == 0 else "#bbbbbb")
        for med in bp["medians"]:
            med.set_color("black")
        ax.set_yticks(pos, labels)
        ax.set(title=title.replace("`", ""), xlabel="Wall time (ms)")
    ps.save(fig, str(R / "figures/headless_boxplot"))


def main():
    long, fails = load()
    (R / "tables").mkdir(exist_ok=True)
    long.to_csv(R / "tables/runs_long.csv", index=False)
    hangs = hang_table()
    hangs.to_csv(R / "tables/print_hangs.csv", index=False)
    print(hangs.to_string())
    res = compare(long)
    res.to_csv(R / "tables/comparison.csv", index=False)
    main_metrics = ["ready_ms", "launch_ready_ms", "export_wall_ms", "print_wall_ms"]
    phase_metrics = ["boot_ms", "pre_invoke_ms", "rust_render_ms", "ipc_ms", "dom_apply_ms", "mathjax_ms",
                     "mermaid_ms", "post_layout_ms"]
    mem_metrics = ["rss_main_mb", "rss_web_mb", "export_rss_mb"]
    for name, ms in [("main", main_metrics), ("phases", phase_metrics), ("memory", mem_metrics)]:
        (R / f"tables/{name}.md").write_text(md_table(res, ms, NAMES) + "\n")
    # sanity: same number of rendered nodes in both arms
    chk = long[long.metric.isin(["math_nodes", "mermaid_svgs"])].groupby(
        ["doc", "metric", "arm"]).value.agg(["min", "max"]).unstack("arm")
    chk.to_csv(R / "tables/render_sanity.csv")
    print(chk)
    print("failed (non-warm-up) runs:", len(fails))
    if len(fails):
        print(fails[["measure", "arm", "doc", "rep", "exit"]])
    counts = long[long.metric.isin(["ready_ms", "export_wall_ms", "print_wall_ms"])].groupby(
        ["metric", "doc", "arm"]).size()
    print(counts.to_string())
    print(res.marker.value_counts().to_string())
    figures(long)


if __name__ == "__main__":
    main()
