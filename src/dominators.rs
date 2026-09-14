//! Dominator trees with the Cooper–Harvey–Kennedy iterative algorithm
//! ("A Simple, Fast Dominance Algorithm", 2001).

use std::collections::VecDeque;

pub const UNREACHABLE: u32 = u32::MAX;

/// A directed graph in compressed sparse row form.
#[derive(Clone, Debug)]
pub struct Graph {
    offsets: Vec<u32>,
    targets: Vec<u32>,
}

impl Graph {
    pub fn from_edges(node_count: usize, edges: &[(u32, u32)]) -> Graph {
        let mut offsets = vec![0u32; node_count + 1];
        for &(from, to) in edges {
            assert!(
                (from as usize) < node_count && (to as usize) < node_count,
                "edge out of range"
            );
            offsets[from as usize + 1] += 1;
        }
        for i in 0..node_count {
            offsets[i + 1] += offsets[i];
        }
        let mut fill = offsets.clone();
        let mut targets = vec![0u32; edges.len()];
        for &(from, to) in edges {
            targets[fill[from as usize] as usize] = to;
            fill[from as usize] += 1;
        }
        Graph { offsets, targets }
    }

    pub fn node_count(&self) -> usize {
        self.offsets.len() - 1
    }

    pub fn edge_count(&self) -> usize {
        self.targets.len()
    }

    pub fn successors(&self, v: u32) -> &[u32] {
        &self.targets[self.offsets[v as usize] as usize..self.offsets[v as usize + 1] as usize]
    }
}

#[derive(Clone, Debug)]
pub struct Dominators {
    /// Immediate dominator per node; the root maps to itself and nodes not
    /// reachable from the root map to [`UNREACHABLE`].
    pub idom: Vec<u32>,
    /// Reachable nodes in reverse postorder. Every node appears after its
    /// immediate dominator.
    pub rpo: Vec<u32>,
}

pub fn dominators(g: &Graph, root: u32) -> Dominators {
    let n = g.node_count();
    let rpo = reverse_postorder(g, root);
    let m = rpo.len();

    // Work in RPO numbering: the intersect step then compares plain integers
    // and the arrays it walks are dense.
    let mut order = vec![UNREACHABLE; n];
    for (i, &v) in rpo.iter().enumerate() {
        order[v as usize] = i as u32;
    }
    let mut pred_off = vec![0u32; m + 1];
    for &v in &rpo {
        for &w in g.successors(v) {
            pred_off[order[w as usize] as usize + 1] += 1;
        }
    }
    for i in 0..m {
        pred_off[i + 1] += pred_off[i];
    }
    let mut fill = pred_off.clone();
    let mut preds = vec![0u32; pred_off[m] as usize];
    for &v in &rpo {
        let from = order[v as usize];
        for &w in g.successors(v) {
            let to = order[w as usize] as usize;
            preds[fill[to] as usize] = from;
            fill[to] += 1;
        }
    }

    let mut idom = vec![UNREACHABLE; m];
    if m > 0 {
        idom[0] = 0;
    }
    let mut changed = true;
    while changed {
        changed = false;
        for b in 1..m {
            let mut new_idom = UNREACHABLE;
            for &p in &preds[pred_off[b] as usize..pred_off[b + 1] as usize] {
                if idom[p as usize] == UNREACHABLE {
                    continue;
                }
                new_idom = if new_idom == UNREACHABLE {
                    p
                } else {
                    intersect(&idom, p, new_idom)
                };
            }
            if idom[b] != new_idom {
                idom[b] = new_idom;
                changed = true;
            }
        }
    }

    let mut out = vec![UNREACHABLE; n];
    for (i, &d) in idom.iter().enumerate() {
        out[rpo[i] as usize] = rpo[d as usize];
    }
    Dominators { idom: out, rpo }
}

fn intersect(idom: &[u32], mut a: u32, mut b: u32) -> u32 {
    while a != b {
        while a > b {
            a = idom[a as usize];
        }
        while b > a {
            b = idom[b as usize];
        }
    }
    a
}

fn reverse_postorder(g: &Graph, root: u32) -> Vec<u32> {
    let n = g.node_count();
    let mut post = Vec::with_capacity(n);
    if n == 0 {
        return post;
    }
    let mut visited = vec![false; n];
    let mut stack: Vec<(u32, u32)> = vec![(root, 0)];
    visited[root as usize] = true;
    while let Some(top) = stack.last_mut() {
        let (v, next) = *top;
        let succ = g.successors(v);
        if (next as usize) < succ.len() {
            top.1 += 1;
            let w = succ[next as usize];
            if !visited[w as usize] {
                visited[w as usize] = true;
                stack.push((w, 0));
            }
        } else {
            post.push(v);
            stack.pop();
        }
    }
    post.reverse();
    post
}

impl Dominators {
    /// Total weight of each node's dominator subtree (0 for unreachable nodes).
    pub fn subtree_weights(&self, weights: &[u64]) -> Vec<u64> {
        let mut acc = vec![0u64; self.idom.len()];
        for &v in &self.rpo {
            acc[v as usize] = weights[v as usize];
        }
        for &v in self.rpo.iter().skip(1).rev() {
            let d = self.idom[v as usize];
            acc[d as usize] += acc[v as usize];
        }
        acc
    }

    /// Children lists of the dominator tree in CSR form: `(offsets, children)`.
    pub fn tree_children(&self) -> (Vec<u32>, Vec<u32>) {
        let n = self.idom.len();
        let mut offsets = vec![0u32; n + 1];
        for &v in self.rpo.iter().skip(1) {
            offsets[self.idom[v as usize] as usize + 1] += 1;
        }
        for i in 0..n {
            offsets[i + 1] += offsets[i];
        }
        let mut fill = offsets.clone();
        let mut children = vec![0u32; self.rpo.len().saturating_sub(1)];
        for &v in self.rpo.iter().skip(1) {
            let d = self.idom[v as usize] as usize;
            children[fill[d] as usize] = v;
            fill[d] += 1;
        }
        (offsets, children)
    }
}

/// The baseline the dominator method replaces: for every reachable node, delete
/// it, re-run a BFS from the root and measure the weight that disappeared.
/// O(n·(n+e)).
pub fn brute_force_exclusive(g: &Graph, root: u32, weights: &[u64]) -> Vec<u64> {
    let n = g.node_count();
    let mut out = vec![0u64; n];
    if n == 0 {
        return out;
    }
    let mut stamp = vec![0u32; n];
    let mut queue = VecDeque::new();
    let mut bfs = |skip: u32, epoch: u32, stamp: &mut [u32]| -> (u64, Vec<u32>) {
        let mut reached = Vec::new();
        let mut total = 0u64;
        if skip == root {
            return (0, reached);
        }
        stamp[root as usize] = epoch;
        queue.push_back(root);
        while let Some(v) = queue.pop_front() {
            total += weights[v as usize];
            if skip == UNREACHABLE {
                reached.push(v);
            }
            for &w in g.successors(v) {
                if w != skip && stamp[w as usize] != epoch {
                    stamp[w as usize] = epoch;
                    queue.push_back(w);
                }
            }
        }
        (total, reached)
    };
    let (full, reachable) = bfs(UNREACHABLE, 1, &mut stamp);
    for (i, &d) in reachable.iter().enumerate() {
        let (without, _) = bfs(d, i as u32 + 2, &mut stamp);
        out[d as usize] = full - without;
    }
    out
}
