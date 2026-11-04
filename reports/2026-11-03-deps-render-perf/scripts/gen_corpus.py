#!/usr/bin/env python3
"""Generate the synthetic benchmark corpus (deterministic, seed 20261103).

Usage: gen_corpus.py <out-dir>

Documents (13):
  prose-{S,M,L}    ~2 KB / ~100 KB / ~1 MB of headings, paragraphs, lists,
                   tables, code fences and quotes. No math, no diagrams.
  math-{S,M,L}     20 / 500 / 3000 equations (60 % inline, 40 % display),
                   separated by short prose.
  mermaid-{S,M,L}  3 / 20 / 60 small diagrams (flowchart, sequence, class,
                   state, pie, gantt in rotation, 6-10 elements each).
  mermaid-XL       one flowchart with 300 nodes, ~450 edges, 6 subgraphs.
  mixed-{S,M,L}    prose sized as prose-*, plus 5 / 150 / 1000 equations and
                   1 / 8 / 20 small diagrams interleaved.
"""
import random
import sys
from pathlib import Path

SEED = 20261103
WORDS = (
    "the of and to in is that for on with as by this be are from at an which "
    "solution population objective front pareto decision variable bilevel leader "
    "follower constraint feasible dominance archive crossover mutation operator "
    "selection convergence diversity indicator hypervolume budget evaluation "
    "instance benchmark landscape gradient surrogate kernel parameter sample "
    "estimate distribution reference neighborhood offspring parent fitness rank "
    "metric layout render document section figure table result method approach"
).split()


class Gen:
    def __init__(self, seed):
        self.r = random.Random(seed)

    def sentence(self, lo=8, hi=22):
        ws = [self.r.choice(WORDS) for _ in range(self.r.randint(lo, hi))]
        s = " ".join(ws)
        # sprinkle inline markup
        if self.r.random() < 0.3:
            i = self.r.randrange(len(ws))
            ws[i] = f"**{ws[i]}**"
            s = " ".join(ws)
        if self.r.random() < 0.2:
            s += f" ([ref](https://example.org/{self.r.choice(WORDS)}))"
        if self.r.random() < 0.2:
            s += f" `{self.r.choice(WORDS)}_{self.r.randint(0, 99)}`"
        return s[0].upper() + s[1:] + "."

    def paragraph(self):
        return " ".join(self.sentence() for _ in range(self.r.randint(3, 6)))

    def heading(self, level):
        return "#" * level + " " + " ".join(self.r.choice(WORDS) for _ in range(self.r.randint(2, 5))).title()

    def bullet_list(self):
        n = self.r.randint(3, 6)
        mark = "-" if self.r.random() < 0.6 else "1."
        return "\n".join(f"{mark} {self.sentence(4, 12)}" for _ in range(n))

    def table(self):
        cols = self.r.randint(3, 5)
        rows = self.r.randint(4, 8)
        head = "| " + " | ".join(self.r.choice(WORDS).title() for _ in range(cols)) + " |"
        sep = "|" + "---|" * cols
        body = [
            "| " + " | ".join(
                f"{self.r.uniform(0, 100):.3f}" if c else self.r.choice(WORDS) for c in range(cols)
            ) + " |"
            for _ in range(rows)
        ]
        return "\n".join([head, sep, *body])

    def code(self):
        lang = self.r.choice(["rust", "python", "julia", "javascript", "bash"])
        lines = []
        for i in range(self.r.randint(5, 14)):
            a, b = self.r.choice(WORDS), self.r.choice(WORDS)
            lines.append(f"    let {a}_{i} = {b}({self.r.randint(0, 999)}, \"{self.r.choice(WORDS)}\");")
        return f"```{lang}\n" + "\n".join(lines) + "\n```"

    def quote(self):
        return "> " + self.sentence(10, 25)

    def prose_block(self):
        k = self.r.random()
        if k < 0.55:
            return self.paragraph()
        if k < 0.70:
            return self.bullet_list()
        if k < 0.80:
            return self.table()
        if k < 0.92:
            return self.code()
        return self.quote()

    # ---- math -------------------------------------------------------------
    def latex(self):
        r = self.r
        v = lambda: r.choice("xyzuvwabc")  # noqa: E731
        g = lambda: r.choice([r"\alpha", r"\beta", r"\gamma", r"\lambda", r"\mu", r"\sigma", r"\theta"])  # noqa: E731
        forms = [
            lambda: rf"\frac{{{v()}^{r.randint(2, 5)} + {g()}}}{{{v()} - {r.randint(1, 9)}}}",
            lambda: rf"\sum_{{i=1}}^{{n}} {g()}_i {v()}_i^{{{r.randint(2, 4)}}}",
            lambda: rf"\int_{{0}}^{{\infty}} e^{{-{g()} {v()}}} \, d{v()}",
            lambda: rf"\min_{{{v()} \in X}} F({v()}) \quad \text{{s.t.}} \quad {v()} \in \arg\min_{{{v()}}} f({v()}, {v()})",
            lambda: rf"\mathbb{{E}}\left[\left\| {v()} - {g()} \right\|_2^2\right] \le {r.randint(1, 9)}\,{g()}",
            lambda: rf"\begin{{pmatrix}} {v()} & {g()} \\ {r.randint(0, 9)} & {v()} \end{{pmatrix}}",
            lambda: rf"\nabla_{{{v()}}} \mathcal{{L}}({v()}, {g()}) = \sqrt{{{v()}^2 + {g()}^2}}",
            lambda: rf"\prod_{{k=1}}^{{m}} \left(1 - \frac{{{g()}_k}}{{{r.randint(2, 9)}}}\right)",
        ]
        return r.choice(forms)()

    def equation(self):
        if self.r.random() < 0.6:
            return f"Here ${self.latex()}$ holds whenever {self.sentence(5, 10).lower()}"
        if self.r.random() < 0.15:
            return (
                "$$\n\\begin{aligned}\n"
                + " \\\\\n".join(f"{self.latex()} &= {self.latex()}" for _ in range(self.r.randint(2, 3)))
                + "\n\\end{aligned}\n$$"
            )
        return f"$$\n{self.latex()}\n$$"

    # ---- mermaid ----------------------------------------------------------
    def small_mermaid(self, i):
        r = self.r
        kind = i % 6
        if kind == 0:
            n = r.randint(6, 10)
            edges = [f"  N{a} -->|{r.choice(WORDS)}| N{r.randint(a + 1, n)}" for a in range(1, n)]
            nodes = [f"  N{k}[{r.choice(WORDS).title()} {k}]" for k in range(1, n + 1)]
            return "flowchart TD\n" + "\n".join(nodes + edges)
        if kind == 1:
            actors = ["Leader", "Follower", "Archive", "Evaluator"]
            msgs = [
                f"  {r.choice(actors)}->>{r.choice(actors)}: {r.choice(WORDS)} {r.randint(0, 99)}"
                for _ in range(r.randint(6, 10))
            ]
            return "sequenceDiagram\n" + "\n".join(msgs)
        if kind == 2:
            cls = [r.choice(WORDS).title() + str(k) for k in range(r.randint(4, 6))]
            body = [f"  class {c} {{\n    +{r.choice(WORDS)} int\n    +{r.choice(WORDS)}()\n  }}" for c in cls]
            rel = [f"  {cls[k]} <|-- {cls[k + 1]}" for k in range(len(cls) - 1)]
            return "classDiagram\n" + "\n".join(body + rel)
        if kind == 3:
            st = [f"S{k}" for k in range(r.randint(5, 8))]
            tr = ["  [*] --> S0"] + [f"  {st[k]} --> {st[k + 1]}: {r.choice(WORDS)}" for k in range(len(st) - 1)]
            return "stateDiagram-v2\n" + "\n".join(tr + [f"  {st[-1]} --> [*]"])
        if kind == 4:
            sl = [f'  "{r.choice(WORDS)} {k}" : {r.randint(5, 60)}' for k in range(r.randint(4, 7))]
            return "pie title " + r.choice(WORDS).title() + "\n" + "\n".join(sl)
        tasks = []
        for k in range(r.randint(5, 8)):
            tasks.append(f"  {r.choice(WORDS).title()} {k} :t{k}, {'2026-01-01' if k == 0 else f'after t{k - 1}'}, {r.randint(1, 9)}d")
        return "gantt\n  dateFormat YYYY-MM-DD\n  section Work\n" + "\n".join(tasks)

    def big_flowchart(self, n=300, groups=6, extra_edges=150):
        r = self.r
        lines = ["flowchart LR"]
        per = n // groups
        for g in range(groups):
            lines.append(f"  subgraph G{g}[Stage {g}]")
            for k in range(g * per, (g + 1) * per):
                lines.append(f"    N{k}[{r.choice(WORDS).title()} {k}]")
            lines.append("  end")
        edges = set()
        for k in range(1, n):
            edges.add((r.randrange(max(0, k - per), k), k))  # tree backbone
        while len(edges) < (n - 1) + extra_edges:
            a = r.randrange(n - 1)
            b = r.randrange(a + 1, min(n, a + 2 * per))
            edges.add((a, b))
        lines += [f"  N{a} --> N{b}" for a, b in sorted(edges)]
        return "\n".join(lines)


def fence(src):
    return f"```mermaid\n{src}\n```"


def prose_doc(g, target_bytes, extras=()):
    """Prose until target_bytes; `extras` are blocks interleaved evenly."""
    blocks = [g.heading(1), g.paragraph()]
    size = sum(len(b) for b in blocks)
    extras = list(extras)
    # estimate how many prose blocks we will emit to interleave extras evenly
    while size < target_bytes:
        if len(blocks) % 12 == 0:
            blocks.append(g.heading(2))
        elif len(blocks) % 5 == 0:
            blocks.append(g.heading(3))
        b = g.prose_block()
        blocks.append(b)
        size += len(b) + 2
    if extras:
        step = max(1, len(blocks) // (len(extras) + 1))
        out = []
        e = iter(extras)
        for i, b in enumerate(blocks):
            out.append(b)
            if i and i % step == 0:
                nxt = next(e, None)
                if nxt is not None:
                    out.append(nxt)
        out.extend(e)  # whatever did not fit
        blocks = out
    return "\n\n".join(blocks) + "\n"


def math_doc(g, n_eq):
    blocks = [g.heading(1), g.paragraph()]
    for k in range(n_eq):
        if k % 25 == 0:
            blocks.append(g.heading(2))
        blocks.append(g.equation())
        if k % 3 == 0:
            blocks.append(g.sentence())
    return "\n\n".join(blocks) + "\n"


def mermaid_doc(g, n):
    blocks = [g.heading(1), g.paragraph()]
    for k in range(n):
        blocks += [g.heading(2), g.sentence(), fence(g.small_mermaid(k))]
    return "\n\n".join(blocks) + "\n"


def main(out):
    out = Path(out)
    out.mkdir(parents=True, exist_ok=True)
    sizes = {"S": 2_000, "M": 100_000, "L": 1_000_000}
    docs = {}
    for s, b in sizes.items():
        docs[f"prose-{s}"] = prose_doc(Gen(SEED + b), b)
    for s, n in {"S": 20, "M": 500, "L": 3000}.items():
        docs[f"math-{s}"] = math_doc(Gen(SEED + 11 + n), n)
    for s, n in {"S": 3, "M": 20, "L": 60}.items():
        docs[f"mermaid-{s}"] = mermaid_doc(Gen(SEED + 7 + n), n)
    g = Gen(SEED + 300)
    docs["mermaid-XL"] = g.heading(1) + "\n\n" + g.paragraph() + "\n\n" + fence(g.big_flowchart()) + "\n"
    for s, (b, ne, nm) in {"S": (2_000, 5, 1), "M": (100_000, 150, 8), "L": (1_000_000, 1000, 20)}.items():
        g = Gen(SEED + 500 + ne)
        extras = [g.equation() for _ in range(ne)] + [fence(g.small_mermaid(k)) for k in range(nm)]
        g.r.shuffle(extras)
        docs[f"mixed-{s}"] = prose_doc(g, b, extras)
    for name, text in docs.items():
        (out / f"{name}.md").write_text(text)
        print(f"{name:12s} {len(text.encode()):>9d} B")


if __name__ == "__main__":
    main(sys.argv[1])
