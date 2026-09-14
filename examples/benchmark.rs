//! Dominator method versus brute-force per-node removal on generated projects.
//!
//!     cargo run --release --example benchmark            # full run
//!     cargo run --release --example benchmark -- --quick # small sizes only
//!
//! Both methods get the same prebuilt TU graphs, run single-threaded, and
//! must produce identical per-node exclusive costs.

use includecost::analyze::{analyze, Options};
use includecost::dominators::{brute_force_exclusive, dominators};
use includecost::generate::{generate, GenConfig};
use includecost::project::Project;
use std::path::Path;
use std::time::{Duration, Instant};

struct Row {
    headers: usize,
    tus: usize,
    edges: usize,
    avg_nodes: f64,
    avg_edges: f64,
    dominator: Duration,
    brute: Duration,
    brute_tus: usize,
    analyze_all_threads: Duration,
}

fn run(headers: usize, tus: usize, brute_limit: usize) -> Row {
    let mut cfg = GenConfig::benchmark(2026);
    cfg.headers = headers;
    cfg.tus = tus;
    let gp = generate(&cfg);
    let (fs, commands) = gp.to_memfs(Path::new("/bench"));
    let project = Project::load(&fs, &commands);
    let graphs: Vec<_> = (0..project.tus.len())
        .map(|tu| project.tu_graph(tu))
        .collect();

    let start = Instant::now();
    let mut fast = Vec::with_capacity(graphs.len());
    for g in &graphs {
        let dom = dominators(&g.graph, 0);
        fast.push(dom.subtree_weights(&g.weights));
    }
    let dominator = start.elapsed();

    let brute_tus = brute_limit.min(graphs.len());
    let start = Instant::now();
    for (g, expected) in graphs.iter().zip(&fast).take(brute_tus) {
        let slow = brute_force_exclusive(&g.graph, 0, &g.weights);
        assert_eq!(&slow, expected, "brute force and dominators disagree");
    }
    let brute = start.elapsed();

    let start = Instant::now();
    let analysis = analyze(&project, &Options::default());
    let analyze_all_threads = start.elapsed();
    assert!(!analysis.headers.is_empty());

    let n = graphs.len().max(1) as f64;
    Row {
        headers,
        tus,
        edges: gp.include_edges,
        avg_nodes: graphs.iter().map(|g| g.nodes.len()).sum::<usize>() as f64 / n,
        avg_edges: graphs.iter().map(|g| g.graph.edge_count()).sum::<usize>() as f64 / n,
        dominator,
        brute,
        brute_tus,
        analyze_all_threads,
    }
}

fn ms(d: Duration) -> String {
    format!("{:.1} ms", d.as_secs_f64() * 1000.0)
}

fn main() {
    let quick = std::env::args().any(|a| a == "--quick");
    let brute_limit: usize = std::env::args()
        .skip_while(|a| a != "--brute-tus")
        .nth(1)
        .map(|v| v.parse().expect("--brute-tus expects a number"))
        .unwrap_or(usize::MAX);
    let scales: &[(usize, usize)] = if quick {
        &[(250, 100), (1000, 400)]
    } else {
        &[(250, 100), (1000, 400), (2500, 1000), (5000, 2000)]
    };
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());

    println!("| headers | TUs | include edges | nodes/TU | edges/TU | dominators (1 thread) | brute force (1 thread) | TUs brute-forced | speedup per TU | full `analyze` ({threads} threads) |");
    println!("|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|");
    for &(headers, tus) in scales {
        let r = run(headers, tus, brute_limit);
        let per_tu_fast = r.dominator.as_secs_f64() / r.tus as f64;
        let per_tu_slow = r.brute.as_secs_f64() / r.brute_tus.max(1) as f64;
        println!(
            "| {} | {} | {} | {:.0} | {:.0} | {} | {} | {} | {:.0}x | {} |",
            r.headers,
            r.tus,
            r.edges,
            r.avg_nodes,
            r.avg_edges,
            ms(r.dominator),
            ms(r.brute),
            r.brute_tus,
            per_tu_slow / per_tu_fast,
            ms(r.analyze_all_threads)
        );
    }
    println!("\nOutputs identical on every brute-forced TU.");
}
