//! Claim 2: a node's dominator-subtree weight equals the bytes that vanish
//! when that node is emptied and the TU is preprocessed again from scratch.

use includecost::analyze::{analyze, tu_costs, Options};
use includecost::dominators::{brute_force_exclusive, dominators};
use includecost::generate::{generate, GenConfig};
use includecost::project::{NodeKey, Project};
use std::collections::HashMap;
use std::path::Path;

fn load(cfg: &GenConfig) -> Project {
    let gp = generate(cfg);
    let (fs, commands) = gp.to_memfs(Path::new("/proj"));
    Project::load(&fs, &commands)
}

#[test]
fn every_node_exclusive_cost_equals_retraversal_difference() {
    let mut checked_nodes = 0;
    let mut unguarded_sites = 0;
    let mut cyclic_tus = 0;
    for seed in 0..12 {
        let project = load(&GenConfig::small(seed));
        for tu in 0..project.tus.len() {
            let g = project.tu_graph(tu);
            let dom = dominators(&g.graph, 0);
            let sub = dom.subtree_weights(&g.weights);
            let full = project.preprocessed_bytes(tu, None);
            assert_eq!(full, sub[0], "seed {seed} tu {tu}: total bytes");
            assert_eq!(dom.rpo.len(), g.nodes.len(), "every node is reachable");
            assert_eq!(brute_force_exclusive(&g.graph, 0, &g.weights), sub);
            if dom.rpo.len() < g.graph.edge_count() + 1 {
                cyclic_tus += usize::from(has_cycle(&g.graph));
            }
            for v in 0..g.nodes.len() as u32 {
                let key = g.key(v);
                let emptied = project.preprocessed_bytes(tu, Some(&key));
                assert_eq!(
                    full - emptied,
                    sub[v as usize],
                    "seed {seed} tu {tu} node {v} key {key:?}"
                );
                checked_nodes += 1;
                unguarded_sites += usize::from(!key.path.is_empty());
            }
        }
    }
    assert!(checked_nodes > 1000, "only {checked_nodes} nodes checked");
    assert!(
        unguarded_sites > 50,
        "generator must produce unguarded inclusion sites"
    );
    assert!(cyclic_tus > 0, "generator must produce include cycles");
}

/// Inclusive bytes by plain BFS: every distinct file reachable in the TU graph
/// from any node of `file`, counted once.
fn distinct_reachable_bytes(
    project: &Project,
    g: &includecost::project::TuGraph,
    file: u32,
) -> u64 {
    let mut seen_node = vec![false; g.nodes.len()];
    let mut queue: Vec<u32> = (0..g.nodes.len() as u32)
        .filter(|&v| v != 0 && g.nodes[v as usize].file == file)
        .collect();
    for &v in &queue {
        seen_node[v as usize] = true;
    }
    let mut files = std::collections::HashSet::new();
    while let Some(v) = queue.pop() {
        files.insert(g.nodes[v as usize].file);
        for &w in g.graph.successors(v) {
            if !seen_node[w as usize] {
                seen_node[w as usize] = true;
                queue.push(w);
            }
        }
    }
    files.iter().map(|&f| project.weight(f)).sum()
}

fn has_cycle(g: &includecost::dominators::Graph) -> bool {
    let n = g.node_count();
    let mut state = vec![0u8; n];
    for s in 0..n as u32 {
        if state[s as usize] != 0 {
            continue;
        }
        let mut stack = vec![(s, 0usize)];
        state[s as usize] = 1;
        while let Some(&mut (v, ref mut i)) = stack.last_mut() {
            if let Some(&w) = g.successors(v).get(*i) {
                *i += 1;
                match state[w as usize] {
                    0 => {
                        state[w as usize] = 1;
                        stack.push((w, 0));
                    }
                    1 => return true,
                    _ => {}
                }
            } else {
                state[v as usize] = 2;
                stack.pop();
            }
        }
    }
    false
}

/// For a guarded header, the per-file number is the same as emptying the file.
/// For an unguarded header it is the sum over its non-nested inclusion sites,
/// which can only under-count emptying every site at once.
#[test]
fn per_file_costs_match_emptying_the_file() {
    let mut guarded_checked = 0;
    for seed in 100..110 {
        let project = load(&GenConfig::small(seed));
        for tu in 0..project.tus.len() {
            let costs = tu_costs(&project, tu);
            let full = project.preprocessed_bytes(tu, None);
            assert_eq!(costs.total, full);
            let g = project.tu_graph(tu);
            let mut sites: HashMap<u32, Vec<NodeKey>> = HashMap::new();
            for v in 1..g.nodes.len() as u32 {
                sites
                    .entry(g.nodes[v as usize].file)
                    .or_default()
                    .push(g.key(v));
            }
            for (file, cost) in &costs.files {
                assert_eq!(cost.expansions as usize, sites[file].len());
                if project.once(*file) {
                    let key = NodeKey {
                        anchor: *file,
                        path: Vec::new(),
                    };
                    assert_eq!(
                        cost.exclusive,
                        full - project.preprocessed_bytes(tu, Some(&key))
                    );
                    guarded_checked += 1;
                } else {
                    let largest_site = sites[file]
                        .iter()
                        .map(|k| full - project.preprocessed_bytes(tu, Some(k)))
                        .max()
                        .unwrap();
                    assert!(cost.exclusive >= largest_site);
                    assert!(cost.exclusive <= full);
                }
            }
        }
    }
    assert!(guarded_checked > 100);
}

#[test]
fn aggregate_is_sum_over_tus_and_independent_of_thread_count() {
    let project = load(&GenConfig::small(42));
    let one = analyze(
        &project,
        &Options {
            threads: 1,
            max_children: 5,
        },
    );
    let many = analyze(
        &project,
        &Options {
            threads: 4,
            max_children: 5,
        },
    );
    assert_eq!(one.headers, many.headers);
    assert_eq!(one.total_bytes, many.total_bytes);

    let mut expected: HashMap<u32, (u64, u32, u64)> = HashMap::new();
    let mut total = 0;
    for tu in 0..project.tus.len() {
        let c = tu_costs(&project, tu);
        total += c.total;
        let g = project.tu_graph(tu);
        for (f, cost) in c.files {
            let e = expected.entry(f).or_default();
            e.0 += cost.exclusive;
            e.1 += 1;
            e.2 += distinct_reachable_bytes(&project, &g, f);
        }
    }
    assert_eq!(one.total_bytes, total);
    assert_eq!(one.headers.len(), expected.len());
    for h in &one.headers {
        let f = project.file_id(&h.path).unwrap();
        assert_eq!(
            (h.exclusive, h.tus, h.inclusive),
            expected[&f],
            "{}",
            h.path.display()
        );
    }
    assert!(one
        .headers
        .windows(2)
        .all(|w| w[0].exclusive >= w[1].exclusive));
}
