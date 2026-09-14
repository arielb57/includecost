//! Loading a compilation database into a resolved file graph, and building
//! the per-translation-unit include graph that dominators run on.

use crate::compdb::{CompileCommand, SearchPaths};
use crate::dominators::Graph;
use crate::fs::SourceProvider;
use crate::resolve::resolve;
use crate::scan::{scan, ScannedFile};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub type FileId = u32;
pub const NO_PARENT: u32 = u32::MAX;

/// Hard cap on nodes in one TU graph. Unguarded headers are expanded once per
/// inclusion site, so a pathological tree of them can grow exponentially.
pub const MAX_TU_NODES: usize = 4_000_000;

#[derive(Clone, Debug)]
pub struct Tu {
    pub file: FileId,
    pub config: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unresolved {
    pub includer: FileId,
    pub line: u32,
    pub spelling: String,
    pub angled: bool,
}

/// Every file reached from any TU, scanned once, with includes resolved once
/// per distinct set of search paths.
pub struct Project {
    pub paths: Vec<PathBuf>,
    index: HashMap<PathBuf, FileId>,
    /// `None` when the file could not be read.
    pub scanned: Vec<Option<ScannedFile>>,
    pub configs: Vec<SearchPaths>,
    /// Per config: file -> resolved target of each include directive, by index.
    pub resolved: Vec<HashMap<FileId, Vec<Option<FileId>>>>,
    pub tus: Vec<Tu>,
    pub unresolved: Vec<Unresolved>,
    pub missing_sources: Vec<PathBuf>,
}

impl Project {
    pub fn load(fs: &dyn SourceProvider, commands: &[CompileCommand]) -> Project {
        let mut p = Project {
            paths: Vec::new(),
            index: HashMap::new(),
            scanned: Vec::new(),
            configs: Vec::new(),
            resolved: Vec::new(),
            tus: Vec::new(),
            unresolved: Vec::new(),
            missing_sources: Vec::new(),
        };
        let mut config_index: HashMap<SearchPaths, u32> = HashMap::new();
        let mut unresolved_seen: HashSet<(FileId, u32)> = HashSet::new();

        for cmd in commands {
            let file = p.intern(fs, &cmd.file);
            if p.scanned[file as usize].is_none() {
                p.missing_sources.push(cmd.file.clone());
                continue;
            }
            let config = *config_index.entry(cmd.search.clone()).or_insert_with(|| {
                p.configs.push(cmd.search.clone());
                p.resolved.push(HashMap::new());
                (p.configs.len() - 1) as u32
            });
            p.tus.push(Tu { file, config });

            let mut work = vec![file];
            while let Some(f) = work.pop() {
                if p.resolved[config as usize].contains_key(&f) {
                    continue;
                }
                let Some(scanned) = &p.scanned[f as usize] else {
                    p.resolved[config as usize].insert(f, Vec::new());
                    continue;
                };
                let includer = p.paths[f as usize].clone();
                let includes = scanned.includes.clone();
                let mut targets = Vec::with_capacity(includes.len());
                for (i, inc) in includes.iter().enumerate() {
                    let search = &p.configs[config as usize];
                    match resolve(fs, &includer, &inc.spelling, inc.angled, search) {
                        Some(path) => {
                            let t = p.intern(fs, &path);
                            if p.scanned[t as usize].is_some() {
                                work.push(t);
                                targets.push(Some(t));
                            } else {
                                targets.push(None);
                            }
                        }
                        None => {
                            if unresolved_seen.insert((f, i as u32)) {
                                p.unresolved.push(Unresolved {
                                    includer: f,
                                    line: inc.line,
                                    spelling: inc.spelling.clone(),
                                    angled: inc.angled,
                                });
                            }
                            targets.push(None);
                        }
                    }
                }
                p.resolved[config as usize].insert(f, targets);
            }
        }
        p
    }

    fn intern(&mut self, fs: &dyn SourceProvider, path: &Path) -> FileId {
        if let Some(&id) = self.index.get(path) {
            return id;
        }
        let id = self.paths.len() as FileId;
        self.paths.push(path.to_path_buf());
        self.index.insert(path.to_path_buf(), id);
        self.scanned.push(fs.read(path).map(|bytes| scan(&bytes)));
        id
    }

    pub fn file_id(&self, path: &Path) -> Option<FileId> {
        self.index.get(path).copied()
    }

    pub fn weight(&self, f: FileId) -> u64 {
        self.scanned[f as usize].as_ref().map_or(0, |s| s.weight)
    }

    pub fn once(&self, f: FileId) -> bool {
        self.scanned[f as usize]
            .as_ref()
            .is_some_and(ScannedFile::once)
    }

    fn targets(&self, config: u32, f: FileId) -> &[Option<FileId>] {
        self.resolved[config as usize]
            .get(&f)
            .map_or(&[], Vec::as_slice)
    }

    /// Builds the include graph of one TU.
    ///
    /// A file that is guarded (or `#pragma once`) is expanded at most once in
    /// a TU, so it is a single node no matter how many routes reach it.
    /// An unguarded file is expanded at every inclusion site, so each site is
    /// its own node. Under these semantics the set of expanded code is exactly
    /// the set of nodes reachable from the root, which is what makes
    /// dominator subtrees equal to removal cost.
    pub fn tu_graph(&self, tu: usize) -> TuGraph {
        let Tu { file: root, config } = self.tus[tu];
        let mut nodes = vec![Node {
            file: root,
            parent: NO_PARENT,
            directive: 0,
        }];
        let mut once_node: HashMap<FileId, u32> = HashMap::new();
        if self.once(root) {
            once_node.insert(root, 0);
        }
        let mut edges: Vec<(u32, u32)> = Vec::new();
        let mut cut_cycles = Vec::new();
        let mut truncated = false;
        let mut work = vec![0u32];

        while let Some(v) = work.pop() {
            let f = nodes[v as usize].file;
            for (i, target) in self.targets(config, f).iter().enumerate() {
                let Some(t) = *target else { continue };
                if self.once(t) {
                    let next = nodes.len() as u32;
                    let id = *once_node.entry(t).or_insert(next);
                    if id == next {
                        nodes.push(Node {
                            file: t,
                            parent: NO_PARENT,
                            directive: 0,
                        });
                        work.push(id);
                    }
                    edges.push((v, id));
                    continue;
                }
                if site_chain_contains(&nodes, v, t) {
                    cut_cycles.push((f, t));
                    continue;
                }
                if nodes.len() >= MAX_TU_NODES {
                    truncated = true;
                    continue;
                }
                let id = nodes.len() as u32;
                nodes.push(Node {
                    file: t,
                    parent: v,
                    directive: i as u32,
                });
                edges.push((v, id));
                work.push(id);
            }
        }
        let graph = Graph::from_edges(nodes.len(), &edges);
        let weights = nodes.iter().map(|n| self.weight(n.file)).collect();
        TuGraph {
            nodes,
            graph,
            weights,
            cut_cycles,
            truncated,
        }
    }

    /// Reference semantics: walks the TU the way the preprocessor would,
    /// expanding directives in order, skipping guarded files already
    /// expanded, and returns the significant bytes produced. The node named
    /// by `emptied`, if any, contributes nothing and includes nothing.
    ///
    /// This shares no code with [`Project::tu_graph`] or the dominator
    /// computation, which is what makes it a useful oracle.
    pub fn preprocessed_bytes(&self, tu: usize, emptied: Option<&NodeKey>) -> u64 {
        let Tu { file: root, config } = self.tus[tu];
        let mut state = Walk {
            project: self,
            config,
            emptied,
            seen: HashSet::new(),
            chain: Vec::new(),
            path: Vec::new(),
            anchor: root,
        };
        if self.once(root) {
            state.seen.insert(root);
        }
        state.expand(root)
    }
}

/// Unguarded files already on the inclusion stack since the nearest guarded
/// ancestor form an include cycle that would never terminate; that edge is cut.
fn site_chain_contains(nodes: &[Node], mut v: u32, file: FileId) -> bool {
    while v != NO_PARENT {
        let node = &nodes[v as usize];
        if node.parent == NO_PARENT {
            return false;
        }
        if node.file == file {
            return true;
        }
        v = node.parent;
    }
    false
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    pub file: FileId,
    /// For an inclusion-site node of an unguarded file: the including node.
    /// [`NO_PARENT`] for the root and for guarded files.
    pub parent: u32,
    pub directive: u32,
}

/// A node identity that does not depend on node numbering: the nearest
/// guarded ancestor (or the root, or the node itself if guarded) plus the
/// directive indices leading from it through unguarded files.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NodeKey {
    pub anchor: FileId,
    pub path: Vec<u32>,
}

pub struct TuGraph {
    pub nodes: Vec<Node>,
    pub graph: Graph,
    pub weights: Vec<u64>,
    /// `(includer, included)` edges dropped because they close a cycle of
    /// unguarded files.
    pub cut_cycles: Vec<(FileId, FileId)>,
    pub truncated: bool,
}

impl TuGraph {
    pub fn key(&self, mut v: u32) -> NodeKey {
        let mut path = Vec::new();
        while self.nodes[v as usize].parent != NO_PARENT {
            path.push(self.nodes[v as usize].directive);
            v = self.nodes[v as usize].parent;
        }
        path.reverse();
        NodeKey {
            anchor: self.nodes[v as usize].file,
            path,
        }
    }
}

struct Walk<'a> {
    project: &'a Project,
    config: u32,
    emptied: Option<&'a NodeKey>,
    seen: HashSet<FileId>,
    chain: Vec<FileId>,
    path: Vec<u32>,
    anchor: FileId,
}

impl Walk<'_> {
    fn expand(&mut self, f: FileId) -> u64 {
        if let Some(key) = self.emptied {
            if key.anchor == self.anchor && key.path == self.path {
                return 0;
            }
        }
        let mut total = self.project.weight(f);
        let targets = self.project.targets(self.config, f);
        for (i, target) in targets.iter().enumerate() {
            let Some(t) = *target else { continue };
            if self.project.once(t) {
                if !self.seen.insert(t) {
                    continue;
                }
                let saved = (
                    std::mem::take(&mut self.chain),
                    std::mem::take(&mut self.path),
                    self.anchor,
                );
                self.anchor = t;
                total += self.expand(t);
                (self.chain, self.path, self.anchor) = saved;
            } else {
                if self.chain.contains(&t) {
                    continue;
                }
                self.chain.push(t);
                self.path.push(i as u32);
                total += self.expand(t);
                self.chain.pop();
                self.path.pop();
            }
        }
        total
    }
}
