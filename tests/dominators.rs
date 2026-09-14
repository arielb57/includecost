//! Claim 1: Cooper–Harvey–Kennedy idoms equal the textbook definition on
//! thousands of random graphs, both acyclic and cyclic.

use includecost::dominators::{brute_force_exclusive, dominators, Graph, UNREACHABLE};
use includecost::generate::Rng;
use std::collections::VecDeque;

fn reachable_without(g: &Graph, root: u32, removed: u32) -> Vec<bool> {
    let mut seen = vec![false; g.node_count()];
    if removed == root {
        return seen;
    }
    seen[root as usize] = true;
    let mut q = VecDeque::from([root]);
    while let Some(v) = q.pop_front() {
        for &w in g.successors(v) {
            if w != removed && !seen[w as usize] {
                seen[w as usize] = true;
                q.push_back(w);
            }
        }
    }
    seen
}

/// d dominates n iff deleting d makes n unreachable (every node dominates
/// itself). The immediate dominator is the strict dominator that is itself
/// dominated by every other strict dominator, i.e. the one with the most
/// dominators.
fn brute_force_idoms(g: &Graph, root: u32) -> Vec<u32> {
    let n = g.node_count();
    let reach = reachable_without(g, root, UNREACHABLE);
    let mut dominates = vec![vec![false; n]; n];
    for d in 0..n {
        if !reach[d] {
            continue;
        }
        let without = reachable_without(g, root, d as u32);
        for v in 0..n {
            dominates[d][v] = reach[v] && (v == d || !without[v]);
        }
    }
    let dom_count: Vec<usize> = (0..n)
        .map(|v| (0..n).filter(|&d| dominates[d][v]).count())
        .collect();
    (0..n)
        .map(|v| {
            if !reach[v] {
                UNREACHABLE
            } else if v as u32 == root {
                root
            } else {
                (0..n)
                    .filter(|&d| d != v && dominates[d][v])
                    .max_by_key(|&d| dom_count[d])
                    .expect("root strictly dominates every other reachable node")
                    as u32
            }
        })
        .collect()
}

/// Every node is reachable from the root: first a random spanning tree, then
/// extra edges. DAGs only add forward edges in a random topological order.
fn random_graph(rng: &mut Rng, cyclic: bool) -> (Graph, u32) {
    let n = 1 + rng.below(40);
    let mut perm: Vec<u32> = (0..n as u32).collect();
    for i in (1..n).rev() {
        perm.swap(i, rng.below(i + 1));
    }
    let root = perm[0];
    let mut edges = Vec::new();
    for i in 1..n {
        let parent = perm[rng.below(i)];
        edges.push((parent, perm[i]));
    }
    let extra = rng.below(2 * n + 1);
    for _ in 0..extra {
        let a = rng.below(n);
        let b = rng.below(n);
        if cyclic || a < b {
            edges.push((perm[a], perm[b]));
        }
    }
    (Graph::from_edges(n, &edges), root)
}

#[test]
fn idoms_match_brute_force_on_2000_random_graphs() {
    let mut rng = Rng::new(0xD0_4115);
    let mut cyclic_with_back_edges = 0;
    for i in 0..2000 {
        let cyclic = i % 2 == 1;
        let (g, root) = random_graph(&mut rng, cyclic);
        let fast = dominators(&g, root);
        let slow = brute_force_idoms(&g, root);
        assert_eq!(fast.idom, slow, "graph {i} (cyclic={cyclic}) root={root}");
        assert_eq!(
            fast.rpo.len(),
            g.node_count(),
            "all nodes are reachable by construction"
        );
        if cyclic && g.edge_count() > g.node_count() {
            cyclic_with_back_edges += 1;
        }
    }
    assert!(
        cyclic_with_back_edges > 500,
        "the cyclic half must actually contain extra edges"
    );
}

#[test]
fn subtree_weights_match_brute_force_removal() {
    let mut rng = Rng::new(77);
    for i in 0..500 {
        let (g, root) = random_graph(&mut rng, i % 2 == 0);
        let weights: Vec<u64> = (0..g.node_count())
            .map(|_| rng.below(1000) as u64)
            .collect();
        let dom = dominators(&g, root);
        assert_eq!(
            dom.subtree_weights(&weights),
            brute_force_exclusive(&g, root, &weights),
            "graph {i}"
        );
    }
}

#[test]
fn irreducible_loop_is_dominated_only_by_its_entry_split() {
    // 0 -> 1, 0 -> 2, 1 <-> 2 (a loop with two entries), 2 -> 3
    let g = Graph::from_edges(4, &[(0, 1), (0, 2), (1, 2), (2, 1), (2, 3)]);
    let d = dominators(&g, 0);
    assert_eq!(d.idom, vec![0, 0, 0, 2]);
}

#[test]
fn unreachable_nodes_have_no_dominator_and_no_weight() {
    let g = Graph::from_edges(4, &[(0, 1), (2, 3), (3, 1)]);
    let d = dominators(&g, 0);
    assert_eq!(d.idom, vec![0, 0, UNREACHABLE, UNREACHABLE]);
    assert_eq!(d.subtree_weights(&[1, 10, 100, 1000]), vec![11, 10, 0, 0]);
    assert_eq!(
        brute_force_exclusive(&g, 0, &[1, 10, 100, 1000]),
        vec![11, 10, 0, 0]
    );
}

#[test]
fn tree_children_invert_idom() {
    let mut rng = Rng::new(5);
    let (g, root) = random_graph(&mut rng, true);
    let d = dominators(&g, root);
    let (off, kids) = d.tree_children();
    let mut seen = 0;
    for v in 0..g.node_count() {
        for &c in &kids[off[v] as usize..off[v + 1] as usize] {
            assert_eq!(d.idom[c as usize], v as u32);
            assert_ne!(c, root);
            seen += 1;
        }
    }
    assert_eq!(seen, g.node_count() - 1);
}
