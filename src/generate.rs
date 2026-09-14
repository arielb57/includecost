//! Deterministic synthetic C++ projects for tests, the benchmark and demos.
//!
//! Headers are grouped into modules. Module `k` may include headers from its
//! own module (lower index only) and from its ancestors in a binary tree of
//! modules, which bounds how much of the project one TU reaches, as in a real
//! layered codebase.

use crate::compdb::{search_paths_from_args, CompileCommand};
use crate::fs::{normalize, MemFs};
use crate::json::quote;
use std::fmt::Write as _;
use std::io;
use std::path::{Path, PathBuf};

/// SplitMix64: tiny, seedable, and good enough for test data.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `0..n`; `n` must be positive.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    pub fn percent(&mut self, p: u32) -> bool {
        self.below(100) < p as usize
    }
}

#[derive(Clone, Debug)]
pub struct GenConfig {
    pub headers: usize,
    pub tus: usize,
    /// Mean number of `#include` lines per header.
    pub includes_per_header: usize,
    pub includes_per_tu: usize,
    pub module_size: usize,
    pub unguarded_percent: u32,
    pub pragma_once_percent: u32,
    pub conditional_percent: u32,
    pub missing_percent: u32,
    /// Chance that a guarded header also includes a later guarded header of
    /// its module, closing an include cycle.
    pub cycle_percent: u32,
    pub seed: u64,
}

impl GenConfig {
    /// A small project that exercises every feature: unguarded headers,
    /// pragma once, conditional and missing includes, and guarded cycles.
    pub fn small(seed: u64) -> GenConfig {
        GenConfig {
            headers: 40,
            tus: 6,
            includes_per_header: 3,
            includes_per_tu: 4,
            module_size: 10,
            unguarded_percent: 15,
            pragma_once_percent: 25,
            conditional_percent: 10,
            missing_percent: 3,
            cycle_percent: 8,
            seed,
        }
    }

    /// The size used by the README benchmark: 5,000 headers, 2,000 TUs and
    /// roughly 60k include edges.
    pub fn benchmark(seed: u64) -> GenConfig {
        GenConfig {
            headers: 5000,
            tus: 2000,
            includes_per_header: 11,
            includes_per_tu: 3,
            module_size: 250,
            unguarded_percent: 2,
            pragma_once_percent: 30,
            conditional_percent: 5,
            missing_percent: 0,
            cycle_percent: 1,
            seed,
        }
    }
}

#[derive(Clone, Debug)]
pub struct GeneratedProject {
    /// `(relative path, contents)`
    pub files: Vec<(String, String)>,
    pub tus: Vec<String>,
    pub include_dirs: Vec<String>,
    pub include_edges: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Style {
    Macro,
    Pragma,
    Unguarded,
}

pub fn generate(cfg: &GenConfig) -> GeneratedProject {
    let mut rng = Rng::new(cfg.seed);
    let module_size = cfg.module_size.max(1);
    let modules = cfg.headers.div_ceil(module_size).max(1);
    let styles: Vec<Style> = (0..cfg.headers)
        .map(|_| {
            if rng.percent(cfg.unguarded_percent) {
                Style::Unguarded
            } else if rng.percent(cfg.pragma_once_percent) {
                Style::Pragma
            } else {
                Style::Macro
            }
        })
        .collect();
    let module_of = |h: usize| h / module_size;
    let ancestors = |mut k: usize| {
        let mut out = Vec::new();
        while k > 0 {
            k = (k - 1) / 2;
            out.push(k);
        }
        out
    };
    let header_in = |rng: &mut Rng, k: usize| {
        let start = k * module_size;
        let end = ((k + 1) * module_size).min(cfg.headers);
        start + rng.below(end - start)
    };

    let mut files = Vec::with_capacity(cfg.headers + cfg.tus);
    let mut include_edges = 0;
    let mut missing_counter = 0;

    for h in 0..cfg.headers {
        let k = module_of(h);
        let local = h - k * module_size;
        let anc = ancestors(k);
        let wanted = if styles[h] == Style::Unguarded {
            rng.below(2)
        } else {
            cfg.includes_per_header / 2 + rng.below(cfg.includes_per_header + 1)
        };
        let mut includes: Vec<(String, bool)> = Vec::new();
        for _ in 0..wanted {
            if rng.percent(cfg.missing_percent) {
                missing_counter += 1;
                includes.push((
                    format!("\"missing/absent{missing_counter}.h\""),
                    rng.percent(cfg.conditional_percent),
                ));
                continue;
            }
            let same_module = local > 0 && (anc.is_empty() || rng.percent(75));
            let target = if same_module {
                k * module_size + rng.below(local)
            } else if !anc.is_empty() {
                let m = anc[rng.below(anc.len())];
                header_in(&mut rng, m)
            } else {
                continue;
            };
            includes.push((
                spell(h, target, module_size),
                rng.percent(cfg.conditional_percent),
            ));
        }
        if styles[h] != Style::Unguarded && rng.percent(cfg.cycle_percent) {
            let end = ((k + 1) * module_size).min(cfg.headers);
            if h + 1 < end {
                let later = h + 1 + rng.below(end - h - 1);
                if styles[later] != Style::Unguarded {
                    includes.push((spell(h, later, module_size), false));
                }
            }
        }
        include_edges += includes.len();
        let name = format!("mod{k}_h{local}");
        files.push((
            header_path(h, module_size),
            header_text(&mut rng, &name, styles[h], &includes),
        ));
    }

    let mut tus = Vec::with_capacity(cfg.tus);
    for t in 0..cfg.tus {
        let k = t % modules;
        let anc = ancestors(k);
        let mut includes = Vec::new();
        if cfg.headers > 0 {
            for _ in 0..cfg.includes_per_tu {
                let m = if anc.is_empty() || rng.percent(70) {
                    k
                } else {
                    anc[rng.below(anc.len())]
                };
                let target = header_in(&mut rng, m);
                let local = target - m * module_size;
                includes.push((
                    format!("<mod{m}/h{local}.h>"),
                    rng.percent(cfg.conditional_percent),
                ));
            }
        }
        include_edges += includes.len();
        let rel = format!("src/tu{t}.cpp");
        let mut text = String::new();
        write_includes(&mut text, &includes);
        let lines = 4 + rng.below(12);
        body(&mut rng, &mut text, &format!("tu{t}"), lines);
        tus.push(rel.clone());
        files.push((rel, text));
    }

    GeneratedProject {
        files,
        tus,
        include_dirs: vec!["include".to_owned()],
        include_edges,
    }
}

fn header_path(h: usize, module_size: usize) -> String {
    format!("include/mod{}/h{}.h", h / module_size, h % module_size)
}

/// Same-module includes use a quoted name relative to the including file, so
/// the includer's directory is searched; others go through `-I`.
fn spell(from: usize, to: usize, module_size: usize) -> String {
    if from / module_size == to / module_size {
        format!("\"h{}.h\"", to % module_size)
    } else {
        format!("<mod{}/h{}.h>", to / module_size, to % module_size)
    }
}

fn write_includes(text: &mut String, includes: &[(String, bool)]) {
    for (i, (spelling, conditional)) in includes.iter().enumerate() {
        if *conditional {
            let _ = writeln!(text, "#ifdef FEATURE_{i}\n#include {spelling}\n#endif");
        } else {
            let _ = writeln!(text, "#include {spelling}");
        }
    }
}

fn header_text(rng: &mut Rng, name: &str, style: Style, includes: &[(String, bool)]) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "// {name}: generated header\n");
    let guard = format!("{}_H", name.to_uppercase());
    match style {
        Style::Macro => {
            let _ = writeln!(text, "#ifndef {guard}\n#define {guard}\n");
        }
        Style::Pragma => text.push_str("#pragma once\n\n"),
        Style::Unguarded => {}
    }
    write_includes(&mut text, includes);
    let lines = if rng.percent(5) {
        40 + rng.below(160)
    } else {
        3 + rng.below(25)
    };
    body(rng, &mut text, name, lines);
    if style == Style::Macro {
        let _ = writeln!(text, "#endif // {guard}");
    }
    text
}

fn body(rng: &mut Rng, text: &mut String, name: &str, lines: usize) {
    for n in 0..lines {
        let _ = match rng.below(7) {
            0 => writeln!(
                text,
                "int {name}_fn{n}(int x) {{ return x * {} + {}; }}",
                rng.below(97),
                rng.below(1000)
            ),
            1 => writeln!(text, "// note {n}: comments do not count toward the weight"),
            2 => writeln!(text),
            3 => writeln!(text, "/* block comment {n}\n   spanning two lines */"),
            4 => writeln!(text, "static const char* {name}_s{n} = \"a//b/*c*/\";"),
            5 => writeln!(
                text,
                "struct {name}_S{n} {{ int a; double b; }};  // trailing"
            ),
            _ => writeln!(
                text,
                "    constexpr long {name}_k{n} = 1'000'{:03};",
                rng.below(1000)
            ),
        };
    }
}

impl GeneratedProject {
    /// Loads the project into memory under `root`, with one compile command
    /// per TU using `-I<root>/include`.
    pub fn to_memfs(&self, root: &Path) -> (MemFs, Vec<CompileCommand>) {
        let mut fs = MemFs::new();
        for (rel, text) in &self.files {
            fs.insert(root.join(rel), text.as_bytes());
        }
        let commands = self
            .tus
            .iter()
            .map(|tu| {
                let args = self.arguments(tu);
                CompileCommand {
                    directory: normalize(root),
                    file: normalize(&root.join(tu)),
                    search: search_paths_from_args(&args, root),
                }
            })
            .collect();
        (fs, commands)
    }

    fn arguments(&self, tu: &str) -> Vec<String> {
        let mut args = vec!["c++".to_owned(), "-std=c++17".to_owned()];
        args.extend(self.include_dirs.iter().map(|d| format!("-I{d}")));
        args.extend(["-c".to_owned(), tu.to_owned()]);
        args
    }

    /// Writes sources and a `compile_commands.json` into `dir`.
    pub fn write_to(&self, dir: &Path) -> io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let dir = dir.canonicalize()?;
        for (rel, text) in &self.files {
            let path = dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, text)?;
        }
        let mut json = String::from("[\n");
        for (i, tu) in self.tus.iter().enumerate() {
            let args: Vec<String> = self.arguments(tu).iter().map(|a| quote(a)).collect();
            let _ = writeln!(
                json,
                "  {{\"directory\": {}, \"file\": {}, \"arguments\": [{}]}}{}",
                quote(&dir.to_string_lossy()),
                quote(tu),
                args.join(", "),
                if i + 1 < self.tus.len() { "," } else { "" }
            );
        }
        json.push_str("]\n");
        let db = dir.join("compile_commands.json");
        std::fs::write(&db, json)?;
        Ok(db)
    }
}
