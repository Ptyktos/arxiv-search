//! Render the benchmark charts from a `bench.py` results JSON with Charton.
//!
//! Usage: `cargo run --release --manifest-path benchmarks/charts/Cargo.toml -- <results.json> [out_dir]`
//!
//! Writes `<name>-light.svg` and `<name>-dark.svg` for each chart.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::{Path, PathBuf};

use charton::prelude::*;
use serde_json::Value;

type Res<T> = Result<T, Box<dyn Error>>;

/// Tools in fixed order; colour follows the tool, never its rank.
const TOOLS: [&str; 3] = ["arxiv-search", "arxiv-mcp-server", "raw-fetch"];

struct Theme {
    name: &'static str,
    surface: &'static str,
    ink: &'static str,
    ink2: &'static str,
    grid: &'static str,
    axis: &'static str,
    /// Validated categorical slots (blue, orange, aqua) for this surface.
    series: [&'static str; 3],
}

const THEMES: [Theme; 2] = [
    Theme {
        name: "light",
        surface: "#fcfcfb",
        ink: "#0b0b0b",
        ink2: "#52514e",
        grid: "#e1e0d9",
        axis: "#c3c2b7",
        series: ["#2a78d6", "#eb6834", "#1baf7a"],
    },
    Theme {
        name: "dark",
        surface: "#1a1a19",
        ink: "#ffffff",
        ink2: "#c3c2b7",
        grid: "#2c2c2a",
        axis: "#383835",
        series: ["#3987e5", "#d95926", "#199e70"],
    },
];

fn short(id: &str) -> String {
    match id {
        "1706.03762v7" => "Attention",
        "1810.04805v2" => "BERT",
        "2106.09685v2" => "LoRA",
        "2310.06825v1" => "Mistral 7B",
        "2005.14165v4" => "GPT-3",
        "2307.09288v2" => "Llama 2",
        "1412.6980v9" => "Adam",
        "0801.1234v2" => "0801.1234 (PDF)",
        other => other,
    }
    .to_string()
}

fn median(mut xs: Vec<f64>) -> Option<f64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_by(f64::total_cmp);
    let n = xs.len();
    Some(if n % 2 == 1 {
        xs[n / 2]
    } else {
        (xs[n / 2 - 1] + xs[n / 2]) / 2.0
    })
}

fn nums(v: &Value) -> Vec<f64> {
    match v {
        Value::Array(a) => a.iter().filter_map(Value::as_f64).collect(),
        other => other.as_f64().into_iter().collect(),
    }
}

/// One grouped horizontal bar chart: `rows` x tools, value per (row, tool).
struct Bars<'a> {
    name: &'a str,
    title: String,
    value_label: &'a str,
    rows: Vec<String>,
    /// tool -> value per row (None = no bar)
    values: BTreeMap<&'a str, Vec<Option<f64>>>,
    log: bool,
}

impl Bars<'_> {
    fn render(&self, out: &Path) -> Res<()> {
        let tools: Vec<&str> = TOOLS
            .iter()
            .copied()
            .filter(|t| self.values.contains_key(t))
            .collect();
        let (mut row, mut value, mut tool) = (Vec::new(), Vec::new(), Vec::new());
        // coord_flip draws the first category at the bottom; feed rows in
        // reverse so the chart reads top-down in corpus order.
        for (i, r) in self.rows.iter().enumerate().rev() {
            for t in &tools {
                if let Some(Some(v)) = self.values[t].get(i) {
                    row.push(r.clone());
                    value.push(*v);
                    tool.push((*t).to_string());
                }
            }
        }
        for theme in &THEMES {
            let palette: Vec<&str> = tools
                .iter()
                .map(|t| theme.series[TOOLS.iter().position(|x| x == t).unwrap_or(0)])
                .collect();
            let ds = Dataset::new()
                .with_column("row", row.clone())?
                .with_column("value", value.clone())?
                .with_column("tool", tool.clone())?;
            let y = if self.log {
                alt::y("value").with_stack("none").with_scale(Scale::Log)
            } else {
                alt::y("value").with_stack("none")
            };
            let height = 140 + 22 * u32::try_from(row.len()).unwrap_or(0);
            Chart::build(ds)?
                .mark_bar()?
                .configure_bar(|b| b.with_stroke(theme.surface).with_stroke_width(1.5).with_width(0.8))
                .encode((alt::x("row"), y, alt::color("tool")))?
                .coord_flip()
                .with_size(820, height)
                .with_title(&self.title)
                .with_x_label("")
                .with_y_label(self.value_label)
                .with_color_label("")
                .configure_theme(|t| {
                    t.with_palette(palette.clone())
                        .with_background_color(theme.surface)
                        .with_grid_color(theme.grid)
                        .with_title_color(theme.ink)
                        .with_label_color(theme.ink2)
                        .with_tick_label_color(theme.ink2)
                        .with_axes_color(theme.axis)
                        .with_tick_color(theme.axis)
                        .with_legend_title_color(theme.ink)
                        .with_legend_label_color(theme.ink)
                })
                .save(out.join(format!("{}-{}.svg", self.name, theme.name)))?;
        }
        Ok(())
    }
}

fn main() -> Res<()> {
    let mut args = std::env::args().skip(1);
    let input = PathBuf::from(args.next().ok_or("usage: charts <results.json> [out_dir]")?);
    let out = args.next().map_or_else(
        || input.parent().unwrap_or(Path::new(".")).join("../charts"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&out)?;
    let data: Value = serde_json::from_str(&std::fs::read_to_string(&input)?)?;
    let papers: Vec<String> = data["corpus"]["papers"]
        .as_array()
        .ok_or("corpus.papers missing")?
        .iter()
        .filter_map(|p| p["id"].as_str().map(str::to_string))
        .collect();
    let empty = Vec::new();
    let rows_of = |scen: &str| data["results"][scen].as_array().unwrap_or(&empty).clone();

    // Median of `key` per (paper, tool) over successful runs.
    let per_paper = |scen: &str, key: &str, tools: &[&'static str]| {
        let rows = rows_of(scen);
        let mut m: BTreeMap<&str, Vec<Option<f64>>> = BTreeMap::new();
        for t in tools {
            let vals = papers
                .iter()
                .map(|p| {
                    median(
                        rows.iter()
                            .filter(|r| r["arm"] == *t && r["paper"] == p.as_str() && r.get("error").is_none())
                            .flat_map(|r| nums(&r[key]))
                            .collect(),
                    )
                })
                .collect();
            m.insert(*t, vals);
        }
        m
    };
    let labels: Vec<String> = papers.iter().map(|p| short(p)).collect();

    if !rows_of("retrieve").is_empty() {
        Bars {
            name: "retrieve-cold",
            title: "First request: paper ID to full text in the agent (median ms, lower is better)".into(),
            value_label: "milliseconds",
            rows: labels.clone(),
            values: per_paper("retrieve", "cold_ms", &TOOLS),
            log: false,
        }
        .render(&out)?;
        Bars {
            name: "retrieve-warm",
            title: "Repeat request, served from local cache (median ms, log scale, lower is better)".into(),
            value_label: "milliseconds (log)",
            rows: labels.clone(),
            values: per_paper("retrieve", "warm_ms", &TOOLS[..2]),
            log: true,
        }
        .render(&out)?;
        Bars {
            name: "tokens",
            title: "Tokens placed in the agent's context per paper (o200k_base, lower is better)".into(),
            value_label: "tokens",
            rows: labels.clone(),
            values: per_paper("retrieve", "response_tokens", &TOOLS),
            log: false,
        }
        .render(&out)?;
        Bars {
            name: "memory",
            title: "Peak server memory while retrieving one paper (median MiB, lower is better)".into(),
            value_label: "MiB",
            rows: labels.clone(),
            values: per_paper("retrieve", "peak_rss_mib", &TOOLS[..2]),
            log: false,
        }
        .render(&out)?;
    }

    let convert = rows_of("convert");
    if !convert.is_empty() {
        // Adam's degraded HTML page converts to almost nothing for both tools.
        let keep: Vec<&String> = papers.iter().filter(|p| p.as_str() != "1412.6980v9").collect();
        let mut values = BTreeMap::new();
        for t in &TOOLS[..2] {
            values.insert(
                *t,
                keep.iter()
                    .map(|p| {
                        convert
                            .iter()
                            .find(|r| r["arm"] == *t && r["paper"] == p.as_str())
                            .and_then(|r| median(nums(&r["samples_ms"])))
                    })
                    .collect(),
            );
        }
        Bars {
            name: "convert",
            title: "HTML/PDF to text on identical local bytes, no network (median ms, log scale)".into(),
            value_label: "milliseconds (log)",
            rows: keep.iter().map(|p| short(p)).collect(),
            values,
            log: true,
        }
        .render(&out)?;
    }

    let search = rows_of("search");
    if !search.is_empty() {
        let mut queries: Vec<String> = Vec::new();
        for r in &search {
            if let Some(q) = r["query"].as_str() {
                if !queries.iter().any(|x| x == q) {
                    queries.push(q.to_string());
                }
            }
        }
        let mut values = BTreeMap::new();
        for t in &TOOLS[..2] {
            values.insert(
                *t,
                queries
                    .iter()
                    .map(|q| {
                        median(
                            search
                                .iter()
                                .filter(|r| r["arm"] == *t && r["query"] == q.as_str())
                                .flat_map(|r| nums(&r["ms"]))
                                .collect(),
                        )
                    })
                    .collect(),
            );
        }
        Bars {
            name: "search",
            title: "Search latency, same query (median ms, lower is better)".into(),
            value_label: "milliseconds",
            rows: queries,
            values,
            log: false,
        }
        .render(&out)?;
    }
    Ok(())
}
