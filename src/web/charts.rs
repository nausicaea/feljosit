//! Server-side SVG charts.
//!
//! The charts are SVG markup that the templates make. They need no JavaScript and no
//! external library. Thus a strict Content Security Policy is possible. This module
//! calculates only the geometry; the templates escape all text.

use serde::Serialize;

pub const WIDTH: i64 = 720;
pub const HEIGHT: i64 = 260;
pub const PAD_LEFT: i64 = 72;
pub const PAD_RIGHT: i64 = 32;
pub const PAD_TOP: i64 = 16;
pub const PAD_BOTTOM: i64 = 40;
/// The maximum number of labels on the x axis. More labels touch each other.
pub const MAX_X_LABELS: usize = 12;
pub const MONTH_LABEL_STEPS: &[usize] = &[1, 2, 3, 6, 12, 24];

#[derive(Debug, Clone, Serialize)]
pub struct Bar {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub css_class: String,
    pub label: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Tick {
    pub y: f64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct XLabel {
    pub x: f64,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Chart {
    pub width: i64,
    pub height: i64,
    pub bars: Vec<Bar>,
    pub ticks: Vec<Tick>,
    pub x_labels: Vec<XLabel>,
    pub zero_y: f64,
    pub left: i64,
    pub right: i64,
    pub line_points: String,
    pub line_dots: Vec<(f64, f64, String)>,
}

pub type Formatter<'a> = &'a dyn Fn(f64) -> String;

fn nice_step(span: f64) -> f64 {
    if span <= 0.0 {
        return 1.0;
    }
    let raw = span / 4.0;
    let magnitude =
        if raw >= 1.0 { 10f64.powi((raw.trunc() as i64).to_string().len() as i32 - 1) } else { 1.0 };
    for factor in [1.0, 2.0, 2.5, 5.0, 10.0] {
        if raw <= factor * magnitude {
            return factor * magnitude;
        }
    }
    10.0 * magnitude
}

fn scale(low: f64, high: f64) -> (f64, f64, f64) {
    let (low, mut high) = (low.min(0.0), high.max(0.0));
    if low == high {
        high = low + 1.0;
    }
    let step = nice_step(high - low);
    ((low / step).floor() * step, (high / step).ceil() * step, step)
}

fn y_of(value: f64, low: f64, high: f64) -> f64 {
    let plot = (HEIGHT - PAD_TOP - PAD_BOTTOM) as f64;
    PAD_TOP as f64 + (high - value) / (high - low) * plot
}

fn ticks(low: f64, high: f64, step: f64, fmt: Formatter<'_>) -> Vec<Tick> {
    let mut out = Vec::new();
    let mut value = low;
    while value <= high + step / 2.0 {
        out.push(Tick { y: y_of(value, low, high), label: fmt(value) });
        value += step;
    }
    out
}

/// The smallest step in months that gives a maximum of `max_labels` labels.
pub fn month_label_step(count: usize, max_labels: usize) -> usize {
    for &step in MONTH_LABEL_STEPS {
        if count.div_ceil(step) <= max_labels {
            return step;
        }
    }
    count.div_ceil(max_labels)
}

/// Labels at the positions where `(index + label_offset) % label_step == 0`.
fn x_labels(positions: &[f64], labels: &[String], label_step: usize, label_offset: usize) -> Vec<XLabel> {
    positions
        .iter()
        .zip(labels)
        .enumerate()
        .filter(|(i, _)| (i + label_offset).is_multiple_of(label_step.max(1)))
        .map(|(_, (x, label))| XLabel { x: *x, label: label.clone() })
        .collect()
}

/// Bars for each label, one bar for each series. `fmt` formats a value.
///
/// The x axis shows each `label_step`-th label. All bars keep their full label in the
/// tooltip.
pub fn grouped_bars(
    labels: &[String],
    series: &[(&str, Vec<i64>)],
    fmt: Formatter<'_>,
    label_step: usize,
    label_offset: usize,
) -> Chart {
    let values: Vec<i64> = series.iter().flat_map(|(_, vs)| vs.iter().copied()).collect();
    let min = values.iter().copied().min().unwrap_or(0) as f64;
    let max = values.iter().copied().max().unwrap_or(0) as f64;
    let (low, high, step) = scale(min, max);
    let plot_width = (WIDTH - PAD_LEFT - PAD_RIGHT) as f64;
    let slot = plot_width / labels.len().max(1) as f64;
    let bar_width = slot * 0.8 / series.len().max(1) as f64;
    let zero = y_of(0.0, low, high);
    let mut bars = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        for (j, (css_class, vs)) in series.iter().enumerate() {
            let value = vs[i] as f64;
            let top = y_of(value.max(0.0), low, high);
            let bottom = y_of(value.min(0.0), low, high);
            bars.push(Bar {
                x: PAD_LEFT as f64 + i as f64 * slot + slot * 0.1 + j as f64 * bar_width,
                y: top,
                width: bar_width,
                height: (bottom - top).max(if vs[i] != 0 { 0.5 } else { 0.0 }),
                css_class: (*css_class).to_string(),
                label: label.clone(),
                title: format!("{label}: {}", fmt(value)),
            });
        }
    }
    let positions: Vec<f64> =
        (0..labels.len()).map(|i| PAD_LEFT as f64 + i as f64 * slot + slot / 2.0).collect();
    Chart {
        width: WIDTH,
        height: HEIGHT,
        bars,
        ticks: ticks(low, high, step, fmt),
        x_labels: x_labels(&positions, labels, label_step, label_offset),
        zero_y: zero,
        left: PAD_LEFT,
        right: WIDTH - PAD_RIGHT,
        line_points: String::new(),
        line_dots: Vec::new(),
    }
}

/// A line with a dot for each value. The x axis labels are as in [`grouped_bars`].
pub fn line(
    labels: &[String],
    values: &[i64],
    fmt: Formatter<'_>,
    label_step: usize,
    label_offset: usize,
) -> Chart {
    let min = values.iter().copied().min().unwrap_or(0) as f64;
    let max = values.iter().copied().max().unwrap_or(0) as f64;
    let (low, high, step) = scale(min, max);
    let plot_width = (WIDTH - PAD_LEFT - PAD_RIGHT) as f64;
    let count = (values.len().max(2) - 1) as f64;
    let points: Vec<(f64, f64)> = values
        .iter()
        .enumerate()
        .map(|(i, v)| (PAD_LEFT as f64 + i as f64 * plot_width / count, y_of(*v as f64, low, high)))
        .collect();
    let positions: Vec<f64> = points.iter().map(|(x, _)| *x).collect();
    Chart {
        width: WIDTH,
        height: HEIGHT,
        bars: Vec::new(),
        ticks: ticks(low, high, step, fmt),
        x_labels: x_labels(&positions, labels, label_step, label_offset),
        zero_y: y_of(0.0, low, high),
        left: PAD_LEFT,
        right: WIDTH - PAD_RIGHT,
        line_points: points.iter().map(|(x, y)| format!("{x:.1},{y:.1}")).collect::<Vec<_>>().join(" "),
        line_dots: points
            .iter()
            .zip(labels)
            .zip(values)
            .map(|(((x, y), label), v)| (*x, *y, format!("{label}: {}", fmt(*v as f64))))
            .collect(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HBar {
    pub label: String,
    pub value_text: String,
    /// Bar length: the largest value is 100 %.
    pub percent: f64,
    /// Part of the total of all values in percent.
    pub share: f64,
}

/// Rows for a horizontal bar list. The largest value is 100 %.
pub fn horizontal_bars(items: &[(String, i64)], fmt: Formatter<'_>) -> Vec<HBar> {
    let largest = items.iter().map(|(_, v)| *v).max().unwrap_or(0);
    let total: i64 = items.iter().map(|(_, v)| *v).sum();
    items
        .iter()
        .map(|(label, value)| HBar {
            label: label.clone(),
            value_text: fmt(*value as f64),
            percent: if largest != 0 { *value as f64 / largest as f64 * 100.0 } else { 0.0 },
            share: if total != 0 { *value as f64 / total as f64 * 100.0 } else { 0.0 },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_steps() {
        assert_eq!(month_label_step(12, MAX_X_LABELS), 1);
        assert_eq!(month_label_step(13, MAX_X_LABELS), 2);
        assert_eq!(month_label_step(24, MAX_X_LABELS), 2);
        assert_eq!(month_label_step(36, MAX_X_LABELS), 3);
        assert_eq!(month_label_step(240, MAX_X_LABELS), 24);
        assert_eq!(month_label_step(1000, MAX_X_LABELS), 84);
    }

    #[test]
    fn scales() {
        assert_eq!(scale(0.0, 0.0), (0.0, 1.0, 1.0));
        assert_eq!(scale(-150.0, 1000.0), (-500.0, 1000.0, 500.0));
        assert_eq!(nice_step(8.0), 2.0);
    }

    #[test]
    fn bars_and_labels() {
        let labels: Vec<String> = (1..=4).map(|i| format!("M{i}")).collect();
        let fmt = |v: f64| format!("{v}");
        let chart = grouped_bars(&labels, &[("a", vec![1, 2, 3, 4]), ("b", vec![0, -1, 0, 0])], &fmt, 2, 1);
        assert_eq!(chart.bars.len(), 8);
        assert_eq!(chart.x_labels.iter().map(|l| l.label.as_str()).collect::<Vec<_>>(), ["M2", "M4"]);
        assert_eq!(chart.bars[1].height, 0.0);
    }
}
