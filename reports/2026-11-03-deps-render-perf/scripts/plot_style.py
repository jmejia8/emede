"""Conference-style matplotlib defaults for experiment reports.

Usage:
    import plot_style as ps
    ps.apply()                 # grayscale + serif + bold titles
    fig, ax = ps.figure()
    ...
    ps.legend(fig, ax)         # inline below (few) / outside right (many)
    ps.save(fig, "figures/name")

Dense curves: pass markevery=0.1 to ax.plot so markers stay readable.
"""

from cycler import cycler
import matplotlib as mpl
import matplotlib.pyplot as plt
from matplotlib import font_manager

SERIF_FONTS = ["C059", "TeX Gyre Termes", "Nimbus Roman", "Times New Roman",
               "Times", "DejaVu Serif"]

# Distinguishable in grayscale: shade + line style + marker change together.
GRAY_COLORS = ["#000000", "#555555", "#888888", "#222222", "#aaaaaa", "#444444"]
LINESTYLES = ["-", "--", "-.", ":", (0, (5, 1, 1, 1, 1, 1)), (0, (1, 1))]
MARKERS = ["o", "s", "^", "D", "v", "x"]

INLINE_LEGEND_MAX = 4


def available_serif():
    installed = {f.name for f in font_manager.fontManager.ttflist}
    return [f for f in SERIF_FONTS if f in installed] or ["serif"]


def apply(color=False, font_size=10):
    """Set rcParams. color=True keeps matplotlib's default color cycle."""
    mpl.rcParams.update(mpl.rcParamsDefault)
    prop = cycler(linestyle=LINESTYLES) + cycler(marker=MARKERS)
    if not color:
        prop = cycler(color=GRAY_COLORS) + prop
    else:
        prop = cycler(color=plt.rcParams["axes.prop_cycle"].by_key()["color"][:6]) + prop
    mpl.rcParams.update({
        "font.family": "serif",
        "font.serif": available_serif(),
        "mathtext.fontset": "stix",          # Times-like math
        "font.size": font_size,
        "axes.titlesize": font_size + 1,
        "axes.titleweight": "bold",
        "figure.titleweight": "bold",
        "axes.labelsize": font_size,
        "axes.prop_cycle": prop,
        "axes.grid": True,
        "grid.color": "#dddddd",
        "grid.linewidth": 0.5,
        "lines.linewidth": 1.4,
        "lines.markersize": 4,
        "legend.frameon": False,
        "figure.constrained_layout.use": True,
        "savefig.dpi": 300,
        "pdf.fonttype": 42,                   # embed TrueType, editable text
    })
    if not color:
        mpl.rcParams["image.cmap"] = "gray_r"


def figure(nrows=1, ncols=1, width=5.0, height=3.4, **kw):
    """Single-column ≈ 3.4in, double-column ≈ 7in wide."""
    return plt.subplots(nrows, ncols, figsize=(width, height),
                        layout="constrained", **kw)


def legend(fig, ax, max_inline=INLINE_LEGEND_MAX, **kw):
    """Borderless legend: one row below the plot when there are few
    entries, outside on the right when there are many."""
    axes = ax if isinstance(ax, (list, tuple)) else getattr(ax, "flat", [ax])
    handles, labels = [], []
    for a in axes:
        for h, l in zip(*a.get_legend_handles_labels()):
            if l not in labels:
                handles.append(h)
                labels.append(l)
    if not handles:
        return None
    if len(handles) <= max_inline:
        return fig.legend(handles, labels, loc="outside lower center",
                          ncol=len(handles), frameon=False, **kw)
    return fig.legend(handles, labels, loc="outside right upper",
                      frameon=False, **kw)


def save(fig, path_stem, formats=("png", "pdf")):
    for ext in formats:
        fig.savefig(f"{path_stem}.{ext}", bbox_inches="tight")
    plt.close(fig)
