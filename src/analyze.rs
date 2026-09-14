//! Per-TU dominator analysis and aggregation across the whole database.

use crate::dominators::dominators;
use crate::project::{FileId, Project, Unresolved, NO_PARENT};
use crate::scan::Guard;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct Options {
    /// Worker threads; 0 means one per available CPU.
    pub threads: usize,
    /// How many dominated children to keep per header.
    pub max_children: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            threads: 0,
            max_children: 3,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct HeaderCost {
    pub path: PathBuf,
    /// Σ over TUs of the bytes that vanish when this header is emptied.
    pub exclusive: u64,
    /// Σ over TUs of the bytes of every distinct file reachable from it.
    pub inclusive: u64,
    /// Translation units in which the header is expanded.
    pub tus: u32,
    /// Σ over TUs of graph nodes for this header: 1 per TU when guarded,
    /// one per inclusion site when not.
    pub expansions: u64,
    /// Included from inside an `#if` block somewhere.
    pub conditional: bool,
    pub guard: Guard,
    /// The largest dominator-tree children: what this header alone brings in.
    pub carries: Vec<(PathBuf, u64)>,
}

impl HeaderCost {
    pub fn ratio(&self) -> f64 {
        if self.inclusive == 0 {
            0.0
        } else {
            self.exclusive as f64 / self.inclusive as f64
        }
    }
}

#[derive(Clone, Debug)]
pub struct Analysis {
    pub translation_units: usize,
    /// Sorted by exclusive bytes, descending.
    pub headers: Vec<HeaderCost>,
    /// Σ over TUs of all preprocessed significant bytes.
    pub total_bytes: u64,
    pub unresolved: Vec<(PathBuf, u32, String, bool)>,
    pub missing_sources: Vec<PathBuf>,
    pub conditional_includes: usize,
    pub computed_includes: usize,
    /// `(includer, included)` pairs cut because they close a cycle of unguarded headers.
    pub unguarded_cycles: Vec<(PathBuf, PathBuf)>,
    pub truncated_tus: usize,
}

impl Analysis {
    pub fn header(&self, path: &std::path::Path) -> Option<&HeaderCost> {
        self.headers.iter().find(|h| h.path == path)
    }
}

/// Per-TU results for one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TuFileCost {
    pub exclusive: u64,
    pub expansions: u64,
}

/// Exclusive cost of every file in one TU, keyed by file, plus dominated
/// children `(parent file, child file) -> bytes`, plus the TU's total bytes.
pub struct TuCosts {
    pub files: HashMap<FileId, TuFileCost>,
    pub children: HashMap<(FileId, FileId), u64>,
    pub total: u64,
    pub cut_cycles: Vec<(FileId, FileId)>,
    pub truncated: bool,
}

pub fn tu_costs(project: &Project, tu: usize) -> TuCosts {
    let g = project.tu_graph(tu);
    let dom = dominators(&g.graph, 0);
    let sub = dom.subtree_weights(&g.weights);
    let (offsets, children) = dom.tree_children();

    let mut files: HashMap<FileId, TuFileCost> = HashMap::new();
    let mut child_bytes: HashMap<(FileId, FileId), u64> = HashMap::new();
    // How many dominator-tree ancestors of the current node belong to each
    // file. An inclusion site nested under another site of the same unguarded
    // file is already inside that site's subtree and must not be added twice.
    let mut open: HashMap<FileId, u32> = HashMap::new();
    let mut stack: Vec<(u32, bool)> = vec![(0, false)];
    while let Some((v, exiting)) = stack.pop() {
        let file = g.nodes[v as usize].file;
        let multi_site = g.nodes[v as usize].parent != NO_PARENT;
        if exiting {
            if multi_site {
                *open.get_mut(&file).expect("entered before exit") -= 1;
            }
            continue;
        }
        let kids = &children[offsets[v as usize] as usize..offsets[v as usize + 1] as usize];
        if v != 0 {
            let entry = files.entry(file).or_default();
            entry.expansions += 1;
            let nested = multi_site && open.get(&file).is_some_and(|&c| c > 0);
            if !nested {
                entry.exclusive += sub[v as usize];
                for &c in kids {
                    let cf = g.nodes[c as usize].file;
                    if cf != file {
                        *child_bytes.entry((file, cf)).or_default() += sub[c as usize];
                    }
                }
            }
        }
        if multi_site {
            *open.entry(file).or_default() += 1;
            stack.push((v, true));
        }
        stack.extend(kids.iter().map(|&c| (c, false)));
    }
    TuCosts {
        files,
        children: child_bytes,
        total: sub[0],
        cut_cycles: g.cut_cycles,
        truncated: g.truncated,
    }
}

#[derive(Default)]
struct Accum {
    exclusive: HashMap<FileId, u64>,
    expansions: HashMap<FileId, u64>,
    /// `(config, file) -> number of TUs`
    presence: HashMap<(u32, FileId), u32>,
    children: HashMap<(FileId, FileId), u64>,
    total: u64,
    cycles: Vec<(FileId, FileId)>,
    truncated: usize,
}

impl Accum {
    fn add(&mut self, config: u32, c: TuCosts) {
        for (f, cost) in c.files {
            *self.exclusive.entry(f).or_default() += cost.exclusive;
            *self.expansions.entry(f).or_default() += cost.expansions;
            *self.presence.entry((config, f)).or_default() += 1;
        }
        for (k, b) in c.children {
            *self.children.entry(k).or_default() += b;
        }
        self.total += c.total;
        self.cycles.extend(c.cut_cycles);
        self.truncated += usize::from(c.truncated);
    }

    fn merge(&mut self, other: Accum) {
        for (f, b) in other.exclusive {
            *self.exclusive.entry(f).or_default() += b;
        }
        for (f, b) in other.expansions {
            *self.expansions.entry(f).or_default() += b;
        }
        for (k, b) in other.presence {
            *self.presence.entry(k).or_default() += b;
        }
        for (k, b) in other.children {
            *self.children.entry(k).or_default() += b;
        }
        self.total += other.total;
        self.cycles.extend(other.cycles);
        self.truncated += other.truncated;
    }
}

pub fn analyze(project: &Project, opts: &Options) -> Analysis {
    let threads = match opts.threads {
        0 => std::thread::available_parallelism().map_or(1, |n| n.get()),
        n => n,
    }
    .min(project.tus.len().max(1));

    let next = AtomicUsize::new(0);
    let merged = Mutex::new(Accum::default());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                let mut local = Accum::default();
                loop {
                    let tu = next.fetch_add(1, Ordering::Relaxed);
                    if tu >= project.tus.len() {
                        break;
                    }
                    local.add(project.tus[tu].config, tu_costs(project, tu));
                }
                merged.lock().expect("worker panicked").merge(local);
            });
        }
    });
    let acc = merged.into_inner().expect("worker panicked");

    let mut inclusive: HashMap<FileId, u64> = HashMap::new();
    for config in 0..project.configs.len() as u32 {
        let wanted: Vec<FileId> = acc
            .presence
            .keys()
            .filter(|(c, _)| *c == config)
            .map(|&(_, f)| f)
            .collect();
        let closure = closure_weights(project, config);
        for f in wanted {
            let tus = acc.presence[&(config, f)] as u64;
            *inclusive.entry(f).or_default() += tus * closure.get(&f).copied().unwrap_or(0);
        }
    }

    let mut conditional = vec![false; project.paths.len()];
    let mut conditional_includes = 0;
    for resolved in &project.resolved {
        for (&f, targets) in resolved {
            let Some(scanned) = &project.scanned[f as usize] else {
                continue;
            };
            for (inc, target) in scanned.includes.iter().zip(targets) {
                if inc.conditional {
                    conditional_includes += 1;
                    if let Some(t) = target {
                        conditional[*t as usize] = true;
                    }
                }
            }
        }
    }
    let computed_includes = project
        .scanned
        .iter()
        .flatten()
        .map(|s| s.computed_includes.len())
        .sum();

    let mut carries: HashMap<FileId, Vec<(FileId, u64)>> = HashMap::new();
    for (&(parent, child), &bytes) in &acc.children {
        carries.entry(parent).or_default().push((child, bytes));
    }

    let mut tus: HashMap<FileId, u32> = HashMap::new();
    for (&(_, f), &n) in &acc.presence {
        *tus.entry(f).or_default() += n;
    }

    let path = |f: FileId| project.paths[f as usize].clone();
    let mut headers: Vec<HeaderCost> = acc
        .exclusive
        .iter()
        .map(|(&f, &exclusive)| {
            let mut kids = carries.remove(&f).unwrap_or_default();
            kids.sort_by(|a, b| {
                b.1.cmp(&a.1)
                    .then_with(|| project.paths[a.0 as usize].cmp(&project.paths[b.0 as usize]))
            });
            kids.truncate(opts.max_children);
            HeaderCost {
                path: path(f),
                exclusive,
                inclusive: inclusive.get(&f).copied().unwrap_or(0),
                tus: tus[&f],
                expansions: acc.expansions[&f],
                conditional: conditional[f as usize],
                guard: project.scanned[f as usize]
                    .as_ref()
                    .map_or(Guard::None, |s| s.guard),
                carries: kids.into_iter().map(|(c, b)| (path(c), b)).collect(),
            }
        })
        .collect();
    headers.sort_by(|a, b| {
        b.exclusive
            .cmp(&a.exclusive)
            .then_with(|| b.inclusive.cmp(&a.inclusive))
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut cycles: Vec<(PathBuf, PathBuf)> = acc
        .cycles
        .iter()
        .map(|&(a, b)| (path(a), path(b)))
        .collect();
    cycles.sort();
    cycles.dedup();

    Analysis {
        translation_units: project.tus.len(),
        headers,
        total_bytes: acc.total,
        unresolved: project
            .unresolved
            .iter()
            .map(
                |Unresolved {
                     includer,
                     line,
                     spelling,
                     angled,
                 }| (path(*includer), *line, spelling.clone(), *angled),
            )
            .collect(),
        missing_sources: project.missing_sources.clone(),
        conditional_includes,
        computed_includes,
        unguarded_cycles: cycles,
        truncated_tus: acc.truncated,
    }
}

/// Weight of the set of distinct files reachable from each file in one
/// config's file graph: the classic "inclusive" number.
///
/// Strongly connected components are condensed (Tarjan, iterative), then each
/// component's reachable set is the union of its successors' bitsets, filled
/// in reverse topological order.
pub fn closure_weights(project: &Project, config: u32) -> HashMap<FileId, u64> {
    let resolved = &project.resolved[config as usize];
    // Only files that something includes need a closure; TU roots are skipped
    // to keep the bitset matrix small.
    let mut local: HashMap<FileId, u32> = HashMap::new();
    let mut files: Vec<FileId> = Vec::new();
    for targets in resolved.values() {
        for &t in targets.iter().flatten() {
            local.entry(t).or_insert_with(|| {
                files.push(t);
                (files.len() - 1) as u32
            });
        }
    }
    let n = files.len();
    let succ: Vec<Vec<u32>> = files
        .iter()
        .map(|f| {
            let mut s: Vec<u32> = resolved.get(f).map_or(Vec::new(), |t| {
                t.iter().flatten().map(|x| local[x]).collect()
            });
            s.sort_unstable();
            s.dedup();
            s
        })
        .collect();

    let comp = tarjan_scc(&succ);
    let comp_count = comp.iter().map(|&c| c as usize + 1).max().unwrap_or(0);
    let words = n.div_ceil(64);
    let mut bits = vec![0u64; comp_count * words];
    let mut members: Vec<Vec<u32>> = vec![Vec::new(); comp_count];
    for (v, &c) in comp.iter().enumerate() {
        members[c as usize].push(v as u32);
    }
    // Tarjan numbers components in reverse topological order: every edge
    // leaves a component for one with a smaller-or-equal number.
    let mut weights = vec![0u64; comp_count];
    for c in 0..comp_count {
        let mut row = vec![0u64; words];
        for &v in &members[c] {
            row[v as usize / 64] |= 1 << (v % 64);
            for &w in &succ[v as usize] {
                let wc = comp[w as usize] as usize;
                if wc != c {
                    let src = &bits[wc * words..(wc + 1) * words];
                    for (r, s) in row.iter_mut().zip(src) {
                        *r |= s;
                    }
                }
            }
        }
        let mut total = 0u64;
        for (wi, &word) in row.iter().enumerate() {
            let mut w = word;
            while w != 0 {
                let bit = w.trailing_zeros() as usize;
                total += project.weight(files[wi * 64 + bit]);
                w &= w - 1;
            }
        }
        weights[c] = total;
        bits[c * words..(c + 1) * words].copy_from_slice(&row);
    }
    files
        .iter()
        .enumerate()
        .map(|(v, &f)| (f, weights[comp[v] as usize]))
        .collect()
}

/// Component index per node; components come out in reverse topological order.
fn tarjan_scc(succ: &[Vec<u32>]) -> Vec<u32> {
    const UNSET: u32 = u32::MAX;
    let n = succ.len();
    let mut index = vec![UNSET; n];
    let mut low = vec![0u32; n];
    let mut on_stack = vec![false; n];
    let mut comp = vec![UNSET; n];
    let mut stack: Vec<u32> = Vec::new();
    let mut next_index = 0u32;
    let mut next_comp = 0u32;
    let mut call: Vec<(u32, usize)> = Vec::new();

    for start in 0..n as u32 {
        if index[start as usize] != UNSET {
            continue;
        }
        call.push((start, 0));
        index[start as usize] = next_index;
        low[start as usize] = next_index;
        next_index += 1;
        stack.push(start);
        on_stack[start as usize] = true;

        while let Some(&mut (v, ref mut pos)) = call.last_mut() {
            if let Some(&w) = succ[v as usize].get(*pos) {
                *pos += 1;
                if index[w as usize] == UNSET {
                    index[w as usize] = next_index;
                    low[w as usize] = next_index;
                    next_index += 1;
                    stack.push(w);
                    on_stack[w as usize] = true;
                    call.push((w, 0));
                } else if on_stack[w as usize] {
                    low[v as usize] = low[v as usize].min(index[w as usize]);
                }
                continue;
            }
            call.pop();
            if let Some(&(parent, _)) = call.last() {
                low[parent as usize] = low[parent as usize].min(low[v as usize]);
            }
            if low[v as usize] == index[v as usize] {
                loop {
                    let w = stack.pop().expect("tarjan stack underflow");
                    on_stack[w as usize] = false;
                    comp[w as usize] = next_comp;
                    if w == v {
                        break;
                    }
                }
                next_comp += 1;
            }
        }
    }
    comp
}
