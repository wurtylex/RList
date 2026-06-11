#!/usr/bin/env python3
"""Render hyperfine JSON results as an SVG bar chart.

Usage: bench_chart.py output.svg results.json [more.json ...]

Bars show mean runtime on a log scale with min/max whiskers. No third party
dependencies, so the benchmark stays reproducible anywhere.
"""

import json
import math
import sys


def fmt_ms(ms: float) -> str:
    return f"{ms:.1f} ms" if ms < 10 else f"{ms:.0f} ms"


def main() -> None:
    out_path, json_paths = sys.argv[1], sys.argv[2:]

    rows = []
    for path in json_paths:
        for r in json.load(open(path))["results"]:
            rows.append(
                {
                    "name": r["command"],
                    "mean": r["mean"] * 1000,
                    "min": r["min"] * 1000,
                    "max": r["max"] * 1000,
                }
            )
    rows.sort(key=lambda r: r["mean"])

    # Layout
    label_w, chart_w, row_h, top, bottom = 230, 430, 30, 14, 30
    width = label_w + chart_w + 70
    height = top + row_h * len(rows) + bottom
    axis_lo, axis_hi = 0.5, 1000.0  # ms, log scale

    def x(ms: float) -> float:
        ms = max(ms, axis_lo)
        frac = (math.log10(ms) - math.log10(axis_lo)) / (
            math.log10(axis_hi) - math.log10(axis_lo)
        )
        return label_w + frac * chart_w

    svg = [
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" '
        f'viewBox="0 0 {width} {height}" font-family="ui-monospace, SFMono-Regular, Menlo, monospace" font-size="12">',
    ]

    # Gridlines at 1, 10, 100, 1000 ms
    for grid in (1, 10, 100, 1000):
        gx = x(grid)
        svg.append(
            f'<line x1="{gx:.1f}" y1="{top - 6}" x2="{gx:.1f}" y2="{height - bottom + 6}" '
            f'stroke="#888" stroke-opacity="0.25" stroke-dasharray="3,3"/>'
        )
        svg.append(
            f'<text x="{gx:.1f}" y="{height - 8}" fill="#888" text-anchor="middle">'
            f"{grid} ms</text>"
        )

    for i, r in enumerate(rows):
        y = top + i * row_h
        bar_y, bar_h = y + 7, 14
        mid = bar_y + bar_h / 2
        x0, x_mean = x(axis_lo), x(r["mean"])
        svg.append(
            f'<text x="{label_w - 10}" y="{mid + 4:.1f}" fill="#888" text-anchor="end">'
            f'{r["name"]}</text>'
        )
        svg.append(
            f'<rect x="{x0:.1f}" y="{bar_y}" width="{x_mean - x0:.1f}" height="{bar_h}" '
            f'rx="2" fill="#2f9e8f"/>'
        )
        svg.append(
            f'<line x1="{x(r["min"]):.1f}" y1="{mid:.1f}" x2="{x(r["max"]):.1f}" y2="{mid:.1f}" '
            f'stroke="#888" stroke-width="1.5"/>'
        )
        for wx in (r["min"], r["max"]):
            svg.append(
                f'<line x1="{x(wx):.1f}" y1="{mid - 4:.1f}" x2="{x(wx):.1f}" y2="{mid + 4:.1f}" '
                f'stroke="#888" stroke-width="1.5"/>'
            )
        svg.append(
            f'<text x="{x(r["max"]) + 8:.1f}" y="{mid + 4:.1f}" fill="#888">'
            f'{fmt_ms(r["mean"])}</text>'
        )

    svg.append("</svg>")
    with open(out_path, "w") as f:
        f.write("\n".join(svg) + "\n")
    print(f"wrote {out_path} ({len(rows)} bars)")


if __name__ == "__main__":
    main()
