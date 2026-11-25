"""Server-side SVG charts.

The charts are SVG markup that Jinja templates make. They need no JavaScript and
no external library. Thus a strict Content Security Policy is possible. This module
calculates only the geometry; the templates escape all text.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass

Formatter = Callable[[float], str]

WIDTH = 720
HEIGHT = 260
PAD_LEFT = 72
PAD_RIGHT = 32
PAD_TOP = 16
PAD_BOTTOM = 40


@dataclass(frozen=True)
class Bar:
    x: float
    y: float
    width: float
    height: float
    css_class: str
    label: str
    title: str


@dataclass(frozen=True)
class Tick:
    y: float
    label: str


@dataclass(frozen=True)
class XLabel:
    x: float
    label: str


@dataclass(frozen=True)
class Chart:
    width: int
    height: int
    bars: list[Bar]
    ticks: list[Tick]
    x_labels: list[XLabel]
    zero_y: float
    left: float = PAD_LEFT
    right: float = WIDTH - PAD_RIGHT
    line_points: str = ""
    line_dots: tuple[tuple[float, float, str], ...] = ()


def _nice_step(span: float) -> float:
    if span <= 0:
        return 1.0
    raw = span / 4
    magnitude = 10 ** (len(str(int(raw))) - 1) if raw >= 1 else 1
    for factor in (1, 2, 2.5, 5, 10):
        if raw <= factor * magnitude:
            return factor * magnitude
    return 10 * magnitude


def _scale(low: float, high: float) -> tuple[float, float, float]:
    low, high = min(low, 0), max(high, 0)
    if low == high:
        high = low + 1
    step = _nice_step(high - low)
    low = (low // step) * step
    high = -((-high) // step) * step
    return low, high, step


def _y(value: float, low: float, high: float) -> float:
    plot = HEIGHT - PAD_TOP - PAD_BOTTOM
    return PAD_TOP + (high - value) / (high - low) * plot


def _ticks(low: float, high: float, step: float, fmt: Formatter) -> list[Tick]:
    ticks = []
    value = low
    while value <= high + step / 2:
        ticks.append(Tick(_y(value, low, high), fmt(value)))
        value += step
    return ticks


def grouped_bars(
    labels: list[str],
    series: list[tuple[str, list[int]]],
    fmt: Formatter,
) -> Chart:
    """Bars for each label, one bar for each series. ``fmt`` formats a value."""
    values = [v for _, vs in series for v in vs] or [0]
    low, high, step = _scale(min(values), max(values))
    plot_width = WIDTH - PAD_LEFT - PAD_RIGHT
    slot = plot_width / max(len(labels), 1)
    bar_width = slot * 0.8 / max(len(series), 1)
    zero = _y(0, low, high)
    bars: list[Bar] = []
    for i, label in enumerate(labels):
        for j, (css_class, vs) in enumerate(series):
            value = vs[i]
            top = _y(max(value, 0), low, high)
            bottom = _y(min(value, 0), low, high)
            bars.append(
                Bar(
                    x=PAD_LEFT + i * slot + slot * 0.1 + j * bar_width,
                    y=top,
                    width=bar_width,
                    height=max(bottom - top, 0.5 if value else 0),
                    css_class=css_class,
                    label=label,
                    title=f"{label}: {fmt(value)}",
                )
            )
    x_labels = [XLabel(PAD_LEFT + i * slot + slot / 2, label) for i, label in enumerate(labels)]
    return Chart(WIDTH, HEIGHT, bars, _ticks(low, high, step, fmt), x_labels, zero)


def line(labels: list[str], values: list[int], fmt: Formatter) -> Chart:
    low, high, step = _scale(min(values or [0]), max(values or [0]))
    plot_width = WIDTH - PAD_LEFT - PAD_RIGHT
    count = max(len(values) - 1, 1)
    points = [(PAD_LEFT + i * plot_width / count, _y(v, low, high)) for i, v in enumerate(values)]
    x_labels = [XLabel(x, label) for (x, _), label in zip(points, labels, strict=True)]
    return Chart(
        WIDTH,
        HEIGHT,
        [],
        _ticks(low, high, step, fmt),
        x_labels,
        _y(0, low, high),
        line_points=" ".join(f"{x:.1f},{y:.1f}" for x, y in points),
        line_dots=tuple(
            (x, y, f"{label}: {fmt(v)}")
            for (x, y), label, v in zip(points, labels, values, strict=True)
        ),
    )


@dataclass(frozen=True)
class HBar:
    label: str
    value_text: str
    #: Bar length: the largest value is 100 %.
    percent: float
    #: Part of the total of all values in percent.
    share: float


def horizontal_bars(items: list[tuple[str, int]], fmt: Formatter) -> list[HBar]:
    """Rows for a horizontal bar list. The largest value is 100 %."""
    largest = max((v for _, v in items), default=0)
    total = sum(v for _, v in items)
    return [
        HBar(
            label,
            fmt(value),
            (value / largest * 100) if largest else 0.0,
            (value / total * 100) if total else 0.0,
        )
        for label, value in items
    ]
