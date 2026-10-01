// SPDX-License-Identifier: MIT OR Apache-2.0

//! The evaluation's two plots, as standalone SVG: a grid of small panels, one row per
//! threshold set and one column per drift amplitude, every panel on the same axes.
//!
//! Series are the simulated cheat classes, each with a fixed colour and dash pattern
//! (so a class looks the same in every panel and plot, and is never told apart by
//! colour alone). The numbers behind every line are in the CSV files beside the plot.

use std::fmt::Write as _;

/// The cheat classes drawn, in legend order. A class keeps its slot whatever else is
/// drawn.
pub const SERIES: [&str; 6] = [
    "flick-aimbot",
    "humanised-aimbot",
    "adaptive-aimbot",
    "fast-adaptive-aimbot",
    "smoothing-aimbot",
    "recoil-macro",
];

/// Dash pattern per slot ("" is solid).
const DASHES: [&str; 6] = ["", "6 3", "2 3", "9 3 2 3", "1 3", "12 4"];

/// One line in a panel.
#[derive(Clone, Debug, PartialEq)]
pub struct Series {
    /// Index into [`SERIES`].
    pub slot: usize,
    /// Points, both coordinates in 0..=1 of the panel.
    pub points: Vec<(f64, f64)>,
    /// Whether to draw the line as steps (right, then up) and without point markers.
    pub steps: bool,
    /// Hover text per point (lines drawn as steps have one for the whole line).
    pub labels: Vec<String>,
}

/// A grid of panels.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// Plot title.
    pub title: String,
    /// Lines under the title: what the axes and rows mean.
    pub subtitle: Vec<String>,
    /// Row labels (threshold sets).
    pub rows: Vec<String>,
    /// Column labels (amplitudes).
    pub columns: Vec<String>,
    /// Tick positions (0..=1) and labels on the x axis.
    pub x_ticks: Vec<(f64, String)>,
    /// A dashed vertical reference line, with its legend text.
    pub reference: Option<(f64, String)>,
    /// A faint curve for comparison (the chance line of a ROC plot), with its legend
    /// text.
    pub baseline: Option<(Vec<(f64, f64)>, String)>,
    /// The series of each panel, `[row][column]`.
    pub panels: Vec<Vec<Vec<Series>>>,
    /// Shown small at the bottom: where the numbers came from.
    pub footer: String,
    /// Recorded in the file's `<metadata>` (git revision, configuration, inputs).
    pub metadata: String,
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renders the grid.
#[must_use]
pub fn render(grid: &Grid) -> String {
    let (pw, ph, left, gap_x, gap_y) = (200.0, 170.0, 76.0, 34.0, 44.0);
    let top = 62.0 + 15.0 * grid.subtitle.len() as f64;
    let (rows, columns) = (grid.rows.len() as f64, grid.columns.len() as f64);
    // Wide enough for the legend whatever the number of columns.
    let width = (left + columns * pw + (columns - 1.0) * gap_x + 24.0).max(980.0);
    let legend_y = top + rows * ph + (rows - 1.0) * gap_y + 44.0;
    let height = legend_y + 66.0;
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}" font-family="system-ui, -apple-system, Segoe UI, sans-serif" font-size="11">
<metadata>{}</metadata>
<style>
  .bg {{ fill: #fcfcfb; }} .t1 {{ fill: #0b0b0b; }} .t2 {{ fill: #52514e; }}
  .grid {{ stroke: #e4e3df; stroke-width: 1; }} .axis {{ stroke: #b9b8b3; stroke-width: 1; fill: none; }}
  .ref {{ stroke: #52514e; stroke-width: 1; stroke-dasharray: 3 3; }}
  .base {{ stroke: #b9b8b3; stroke-width: 1; fill: none; }}
  .s0 {{ stroke: #2a78d6; }} .s1 {{ stroke: #eb6834; }} .s2 {{ stroke: #1baf7a; }}
  .s3 {{ stroke: #eda100; }} .s4 {{ stroke: #e87ba4; }} .s5 {{ stroke: #008300; }}
  .line {{ fill: none; stroke-width: 2; stroke-linejoin: round; stroke-linecap: round; }}
  .dot {{ fill: #fcfcfb; stroke-width: 2; }}
  @media (prefers-color-scheme: dark) {{
    .bg {{ fill: #1a1a19; }} .t1 {{ fill: #ffffff; }} .t2 {{ fill: #c3c2b7; }}
    .grid {{ stroke: #33322f; }} .axis {{ stroke: #5c5b57; }} .ref {{ stroke: #c3c2b7; }} .base {{ stroke: #5c5b57; }}
    .s0 {{ stroke: #3987e5; }} .s1 {{ stroke: #d95926; }} .s2 {{ stroke: #199e70; }}
    .s3 {{ stroke: #c98500; }} .s4 {{ stroke: #d55181; }} .s5 {{ stroke: #008300; }}
    .dot {{ fill: #1a1a19; }}
  }}
</style>
<rect class="bg" x="0" y="0" width="{width}" height="{height}"/>
<text class="t1" x="{left}" y="24" font-size="15" font-weight="600">{}</text>
"#,
        escape(&grid.metadata),
        escape(&grid.title)
    );
    for (i, line) in grid.subtitle.iter().enumerate() {
        let _ = writeln!(
            s,
            r#"<text class="t2" x="{left}" y="{}">{}</text>"#,
            42.0 + 15.0 * i as f64,
            escape(line)
        );
    }
    for (row, row_label) in grid.rows.iter().enumerate() {
        for (column, column_label) in grid.columns.iter().enumerate() {
            let (ox, oy) = (
                left + column as f64 * (pw + gap_x),
                top + row as f64 * (ph + gap_y),
            );
            let _ = writeln!(s, r#"<g transform="translate({ox},{oy})">"#);
            for i in 0..=4 {
                let y = f64::from(i) / 4.0 * ph;
                let _ = writeln!(
                    s,
                    r#"<line class="grid" x1="0" y1="{y}" x2="{pw}" y2="{y}"/>"#
                );
            }
            for (x, _) in &grid.x_ticks {
                let x = x * pw;
                let _ = writeln!(
                    s,
                    r#"<line class="grid" x1="{x:.1}" y1="0" x2="{x:.1}" y2="{ph}"/>"#
                );
            }
            if let Some((curve, _)) = &grid.baseline {
                let points: Vec<String> = curve
                    .iter()
                    .map(|(x, y)| format!("{:.1},{:.1}", x * pw, (1.0 - y) * ph))
                    .collect();
                let _ = writeln!(
                    s,
                    r#"<polyline class="base" points="{}"/>"#,
                    points.join(" ")
                );
            }
            if let Some((x, _)) = &grid.reference {
                let x = x * pw;
                let _ = writeln!(
                    s,
                    r#"<line class="ref" x1="{x:.1}" y1="0" x2="{x:.1}" y2="{ph}"/>"#
                );
            }
            let _ = writeln!(
                s,
                r#"<rect class="axis" x="0" y="0" width="{pw}" height="{ph}"/>"#
            );
            let series = &grid.panels[row][column];
            if series.is_empty() {
                let _ = writeln!(
                    s,
                    r#"<text class="t2" x="{}" y="{}" text-anchor="middle">no sessions</text>"#,
                    pw / 2.0,
                    ph / 2.0
                );
            }
            for line in series {
                let dash = match DASHES[line.slot] {
                    "" => String::new(),
                    d => format!(r#" stroke-dasharray="{d}""#),
                };
                let at =
                    |p: &(f64, f64)| (p.0.clamp(0.0, 1.0) * pw, (1.0 - p.1.clamp(0.0, 1.0)) * ph);
                let mut points: Vec<String> = Vec::new();
                let mut previous: Option<(f64, f64)> = None;
                for (i, p) in line.points.iter().enumerate() {
                    let (x, y) = at(p);
                    // Drop points within half a pixel of the last one drawn.
                    if let Some((px, py)) = previous
                        && i + 1 != line.points.len()
                        && (x - px).abs() < 0.5
                        && (y - py).abs() < 0.5
                    {
                        continue;
                    }
                    if let (true, Some((_, py))) = (line.steps, previous) {
                        points.push(format!("{x:.1},{py:.1}"));
                    }
                    points.push(format!("{x:.1},{y:.1}"));
                    previous = Some((x, y));
                }
                let name = SERIES[line.slot];
                let whole = if line.steps {
                    line.labels.first()
                } else {
                    None
                };
                let _ = writeln!(
                    s,
                    r#"<polyline class="line s{}"{dash} points="{}"><title>{}</title></polyline>"#,
                    line.slot,
                    points.join(" "),
                    escape(whole.map_or(name, String::as_str))
                );
                if !line.steps {
                    for (p, label) in line.points.iter().zip(&line.labels) {
                        let (x, y) = at(p);
                        let _ = writeln!(
                            s,
                            r#"<circle class="dot s{}" cx="{x:.1}" cy="{y:.1}" r="3"><title>{}</title></circle>"#,
                            line.slot,
                            escape(label)
                        );
                    }
                }
            }
            if row == 0 {
                let _ = writeln!(
                    s,
                    r#"<text class="t1" x="{}" y="-10" text-anchor="middle" font-weight="600">{}</text>"#,
                    pw / 2.0,
                    escape(column_label)
                );
            }
            if column == 0 {
                let _ = writeln!(
                    s,
                    r#"<text class="t1" x="-50" y="{0}" text-anchor="middle" font-weight="600" transform="rotate(-90 -50 {0})">{1}</text>"#,
                    ph / 2.0,
                    escape(row_label)
                );
                for (v, label) in [(1.0, "100%"), (0.5, "50%"), (0.0, "0%")] {
                    let _ = writeln!(
                        s,
                        r#"<text class="t2" x="-6" y="{}" text-anchor="end" dominant-baseline="middle">{label}</text>"#,
                        (1.0 - v) * ph
                    );
                }
            }
            if row + 1 == grid.rows.len() {
                for (x, label) in &grid.x_ticks {
                    let _ = writeln!(
                        s,
                        r#"<text class="t2" x="{:.1}" y="{}" text-anchor="middle">{}</text>"#,
                        x * pw,
                        ph + 14.0,
                        escape(label)
                    );
                }
            }
            let _ = writeln!(s, "</g>");
        }
    }
    // Legend: every class, with the same colour and dash as its lines.
    let mut x = left;
    for (slot, name) in SERIES.iter().enumerate() {
        let dash = match DASHES[slot] {
            "" => String::new(),
            d => format!(r#" stroke-dasharray="{d}""#),
        };
        let _ = writeln!(
            s,
            r#"<line class="line s{slot}"{dash} x1="{x}" y1="{legend_y}" x2="{}" y2="{legend_y}"/>
<text class="t1" x="{}" y="{legend_y}" dominant-baseline="middle">{name}</text>"#,
            x + 28.0,
            x + 34.0
        );
        x += 34.0 + 6.4 * name.len() as f64 + 22.0;
    }
    let mut x = left;
    let extra_y = legend_y + 20.0;
    for (class, text) in [
        ("ref", grid.reference.as_ref().map(|r| &r.1)),
        ("base", grid.baseline.as_ref().map(|b| &b.1)),
    ] {
        if let Some(text) = text {
            let _ = writeln!(
                s,
                r#"<line class="{class}" x1="{x}" y1="{extra_y}" x2="{}" y2="{extra_y}"/>
<text class="t2" x="{}" y="{extra_y}" dominant-baseline="middle">{}</text>"#,
                x + 28.0,
                x + 34.0,
                escape(text)
            );
            x += 34.0 + 6.4 * text.len() as f64 + 22.0;
        }
    }
    let _ = writeln!(
        s,
        r#"<text class="t2" x="{left}" y="{}">{}</text>"#,
        legend_y + 44.0,
        escape(&grid.footer)
    );
    let _ = writeln!(s, "</svg>");
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid() -> Grid {
        let line = |slot, steps| Series {
            slot,
            points: vec![(0.0, 0.0), (0.25, 0.5), (0.2501, 0.5001), (1.0, 1.0)],
            steps,
            labels: vec!["a <b> & c".to_owned(); 4],
        };
        Grid {
            title: "Title".into(),
            subtitle: vec!["Rows & columns".into(), "Second line".into()],
            rows: vec!["sim".into(), "human".into()],
            columns: vec!["0.5%".into(), "1%".into(), "2%".into()],
            x_ticks: vec![(0.0, "0".into()), (1.0, "60 s".into())],
            reference: Some((0.25, "target".into())),
            baseline: Some((vec![(0.0, 0.0), (1.0, 1.0)], "chance".into())),
            panels: vec![
                vec![
                    vec![line(0, false), line(5, true)],
                    vec![line(1, false)],
                    vec![],
                ],
                vec![vec![], vec![], vec![line(2, true)]],
            ],
            footer: "git abc".into(),
            metadata: r#"{"git":"abc--def","note":"<x>"}"#.into(),
        }
    }

    #[test]
    fn renders_every_panel_series_and_legend_entry() {
        let svg = render(&grid());
        assert!(svg.starts_with("<svg") && svg.trim_end().ends_with("</svg>"));
        assert_eq!(svg.matches("<g ").count(), 6);
        assert_eq!(svg.matches("<polyline class=\"line").count(), 4);
        assert_eq!(svg.matches("no sessions").count(), 3);
        // Markers on plain lines only, one per point, each with hover text.
        assert_eq!(svg.matches("<circle").count(), 8);
        for name in SERIES {
            assert!(svg.contains(&format!(">{name}</text>")), "{name}");
        }
        // Text is escaped, and the metadata is carried.
        assert!(svg.contains("a &lt;b&gt; &amp; c") && !svg.contains("<b>"));
        assert!(svg.contains(r#"<metadata>{"git":"abc--def","note":"&lt;x&gt;"}</metadata>"#));
        assert!(svg.contains("chance") && svg.contains("target") && svg.contains("git abc"));
        assert!(svg.contains("Rows &amp; columns") && svg.contains("Second line"));
    }

    #[test]
    fn steps_go_right_then_up_and_near_duplicates_are_dropped() {
        let svg = render(&grid());
        let steps = svg.lines().find(|l| l.contains("line s5")).unwrap();
        // (0,0) -> (0.25,0.5): a corner at the new x and the old y. The point half a
        // pixel away is dropped.
        assert!(
            steps.contains(r#"points="0.0,170.0 50.0,170.0 50.0,85.0 200.0,85.0 200.0,0.0""#),
            "{steps}"
        );
        assert!(steps.contains("stroke-dasharray=\"12 4\""));
    }
}
