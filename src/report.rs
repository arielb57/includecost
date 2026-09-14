//! Text and JSON rendering of an [`Analysis`].

use crate::analyze::{Analysis, HeaderCost};
use crate::json::quote;
use crate::scan::Guard;
use std::fmt::Write as _;
use std::path::Path;

/// Human-readable byte count: exact below 1 KiB, one decimal above.
pub fn human_bytes(b: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if b < 1024 {
        return format!("{b} B");
    }
    let mut v = b as f64 / 1024.0;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    format!("{v:.1} {}", UNITS[unit])
}

fn display_path(path: &Path, base: &Path) -> String {
    path.strip_prefix(base)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn shown(analysis: &Analysis, top: usize) -> &[HeaderCost] {
    let n = if top == 0 {
        analysis.headers.len()
    } else {
        top.min(analysis.headers.len())
    };
    &analysis.headers[..n]
}

pub fn text(analysis: &Analysis, top: usize, base: &Path) -> String {
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{} translation units, {} headers, {} preprocessed in total",
        analysis.translation_units,
        analysis.headers.len(),
        human_bytes(analysis.total_bytes)
    );
    let _ = writeln!(
        out,
        "{} unresolved includes, {} conditional includes (treated as taken), {} computed includes (skipped)",
        analysis.unresolved.len(),
        analysis.conditional_includes,
        analysis.computed_includes
    );
    out.push('\n');
    let _ = writeln!(
        out,
        "{:>4}  {:>10}  {:>10}  {:>6}  {:>5}  header",
        "rank", "exclusive", "inclusive", "excl%", "TUs"
    );
    for (i, h) in shown(analysis, top).iter().enumerate() {
        let mut tags = Vec::new();
        if h.guard == Guard::None {
            tags.push(format!("unguarded, {} expansions", h.expansions));
        }
        if h.conditional {
            tags.push("conditional".to_owned());
        }
        let tags = if tags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", tags.join("; "))
        };
        let _ = writeln!(
            out,
            "{:>4}  {:>10}  {:>10}  {:>5.1}%  {:>5}  {}{}",
            i + 1,
            human_bytes(h.exclusive),
            human_bytes(h.inclusive),
            100.0 * h.ratio(),
            h.tus,
            display_path(&h.path, base),
            tags
        );
        if !h.carries.is_empty() {
            let kids: Vec<String> = h
                .carries
                .iter()
                .map(|(p, b)| format!("{} {}", display_path(p, base), human_bytes(*b)))
                .collect();
            let _ = writeln!(out, "{:>44}carries: {}", "", kids.join(", "));
        }
    }
    if shown(analysis, top).len() < analysis.headers.len() {
        let _ = writeln!(
            out,
            "... {} more (use --top 0 for all)",
            analysis.headers.len() - shown(analysis, top).len()
        );
    }

    if !analysis.unresolved.is_empty() {
        let _ = writeln!(out, "\nunresolved includes:");
        for (includer, line, spelling, angled) in analysis.unresolved.iter().take(10) {
            let (open, close) = if *angled { ('<', '>') } else { ('"', '"') };
            let _ = writeln!(
                out,
                "  {}:{}: {open}{spelling}{close}",
                display_path(includer, base),
                line
            );
        }
        if analysis.unresolved.len() > 10 {
            let _ = writeln!(
                out,
                "  ... {} more (see --json)",
                analysis.unresolved.len() - 10
            );
        }
    }
    for src in &analysis.missing_sources {
        let _ = writeln!(
            out,
            "warning: source file not found, TU skipped: {}",
            src.display()
        );
    }
    for (a, b) in &analysis.unguarded_cycles {
        let _ = writeln!(
            out,
            "warning: include cycle through unguarded headers cut at {} -> {}",
            display_path(a, base),
            display_path(b, base)
        );
    }
    if analysis.truncated_tus > 0 {
        let _ = writeln!(
            out,
            "warning: {} TUs exceeded the node limit; their costs are incomplete",
            analysis.truncated_tus
        );
    }
    out
}

pub fn json(analysis: &Analysis, top: usize, base: &Path) -> String {
    let p = |path: &Path| quote(&display_path(path, base));
    let mut out = String::from("{\n");
    let _ = writeln!(
        out,
        "  \"translation_units\": {},",
        analysis.translation_units
    );
    let _ = writeln!(out, "  \"headers_total\": {},", analysis.headers.len());
    let _ = writeln!(out, "  \"total_bytes\": {},", analysis.total_bytes);
    let _ = writeln!(
        out,
        "  \"conditional_includes\": {},",
        analysis.conditional_includes
    );
    let _ = writeln!(
        out,
        "  \"computed_includes\": {},",
        analysis.computed_includes
    );
    out.push_str("  \"headers\": [");
    for (i, h) in shown(analysis, top).iter().enumerate() {
        let guard = match h.guard {
            Guard::None => "none",
            Guard::Macro => "macro",
            Guard::PragmaOnce => "pragma_once",
        };
        let carries: Vec<String> = h
            .carries
            .iter()
            .map(|(c, b)| format!("{{\"path\": {}, \"bytes\": {b}}}", p(c)))
            .collect();
        let _ = write!(
            out,
            "{}\n    {{\"path\": {}, \"exclusive_bytes\": {}, \"inclusive_bytes\": {}, \"ratio\": {:.6}, \"tus\": {}, \"expansions\": {}, \"guard\": \"{guard}\", \"conditional\": {}, \"carries\": [{}]}}",
            if i == 0 { "" } else { "," },
            p(&h.path),
            h.exclusive,
            h.inclusive,
            h.ratio(),
            h.tus,
            h.expansions,
            h.conditional,
            carries.join(", ")
        );
    }
    out.push_str("\n  ],\n  \"unresolved\": [");
    for (i, (includer, line, spelling, angled)) in analysis.unresolved.iter().enumerate() {
        let _ = write!(
            out,
            "{}\n    {{\"includer\": {}, \"line\": {line}, \"spelling\": {}, \"angled\": {angled}}}",
            if i == 0 { "" } else { "," },
            p(includer),
            quote(spelling)
        );
    }
    out.push_str("\n  ],\n  \"missing_sources\": [");
    let missing: Vec<String> = analysis
        .missing_sources
        .iter()
        .map(|m| quote(&m.to_string_lossy()))
        .collect();
    out.push_str(&missing.join(", "));
    out.push_str("],\n  \"unguarded_cycles\": [");
    let cycles: Vec<String> = analysis
        .unguarded_cycles
        .iter()
        .map(|(a, b)| format!("[{}, {}]", p(a), p(b)))
        .collect();
    out.push_str(&cycles.join(", "));
    let _ = write!(
        out,
        "],\n  \"truncated_tus\": {}\n}}\n",
        analysis.truncated_tus
    );
    out
}
