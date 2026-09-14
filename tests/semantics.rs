//! Claims 3, 4 and 7: guard semantics, partial guards, and include cycles.

use includecost::analyze::{analyze, tu_costs, Options};
use includecost::compdb::{CompileCommand, SearchPaths};
use includecost::dominators::dominators;
use includecost::project::Project;
use includecost::scan::{scan, Guard};
use includecost::MemFs;
use std::path::{Path, PathBuf};

fn project(files: &[(&str, &str)], tus: &[&str]) -> Project {
    let mut fs = MemFs::new();
    for (path, text) in files {
        fs.insert(format!("/p/{path}"), *text);
    }
    let commands: Vec<CompileCommand> = tus
        .iter()
        .map(|tu| CompileCommand {
            directory: PathBuf::from("/p"),
            file: PathBuf::from(format!("/p/{tu}")),
            search: SearchPaths::default(),
        })
        .collect();
    Project::load(&fs, &commands)
}

fn weight(text: &str) -> u64 {
    scan(text.as_bytes()).weight
}

fn guarded(name: &str, body: &str) -> String {
    format!("#ifndef {name}_H\n#define {name}_H\n{body}\n#endif\n")
}

const A: &str = "#include \"b.h\"\n#include \"c.h\"\nint main() { return 0; }\n";

fn diamond(d: &str) -> (Project, [u64; 4]) {
    let b = guarded("B", "#include \"d.h\"\nint b;");
    let c = guarded("C", "#include \"d.h\"\nint c;");
    let p = project(
        &[("a.cpp", A), ("b.h", &b), ("c.h", &c), ("d.h", d)],
        &["a.cpp"],
    );
    (p, [weight(A), weight(&b), weight(&c), weight(d)])
}

fn cost(p: &Project, file: &str) -> includecost::analyze::TuFileCost {
    let id = p.file_id(Path::new(&format!("/p/{file}"))).unwrap();
    tu_costs(p, 0).files[&id].clone()
}

#[test]
fn guarded_diamond_counts_d_once_and_credits_it_to_the_root() {
    let d = guarded("D", "struct D { int big[100]; };\nint d1;\nint d2;");
    let (p, [wa, wb, wc, wd]) = diamond(&d);
    assert_eq!(scan(d.as_bytes()).guard, Guard::Macro);
    assert_eq!(p.preprocessed_bytes(0, None), wa + wb + wc + wd);

    let g = p.tu_graph(0);
    assert_eq!(g.nodes.len(), 4, "d.h is one node");
    let dom = dominators(&g.graph, 0);
    let d_id = p.file_id(Path::new("/p/d.h")).unwrap();
    let d_node = g.nodes.iter().position(|n| n.file == d_id).unwrap();
    assert_eq!(
        dom.idom[d_node], 0,
        "D is immediately dominated by A, not B or C"
    );

    assert_eq!(cost(&p, "b.h").exclusive, wb);
    assert_eq!(cost(&p, "c.h").exclusive, wc);
    assert_eq!(cost(&p, "d.h").exclusive, wd);
    assert_eq!(cost(&p, "d.h").expansions, 1);

    let analysis = analyze(&p, &Options::default());
    let b = analysis.header(Path::new("/p/b.h")).unwrap();
    assert_eq!(b.inclusive, wb + wd, "inclusive still counts D under B");
    assert!(b.carries.is_empty(), "B dominates nothing");
}

#[test]
fn pragma_once_diamond_behaves_like_a_guard() {
    let d = "#pragma once\nint d1;\nint d2;\n";
    let (p, [wa, wb, wc, wd]) = diamond(d);
    assert_eq!(scan(d.as_bytes()).guard, Guard::PragmaOnce);
    assert_eq!(p.preprocessed_bytes(0, None), wa + wb + wc + wd);
    assert_eq!(cost(&p, "b.h").exclusive, wb);
    assert_eq!(cost(&p, "d.h").exclusive, wd);
}

#[test]
fn unguarded_diamond_counts_d_twice_and_credits_each_copy_to_its_includer() {
    let d = "int d1;\nint d2;\n";
    let (p, [wa, wb, wc, wd]) = diamond(d);
    assert_eq!(scan(d.as_bytes()).guard, Guard::None);
    assert_eq!(p.preprocessed_bytes(0, None), wa + wb + wc + 2 * wd);
    assert_eq!(p.tu_graph(0).nodes.len(), 5, "one node per inclusion site");
    assert_eq!(cost(&p, "b.h").exclusive, wb + wd);
    assert_eq!(cost(&p, "c.h").exclusive, wc + wd);
    assert_eq!(cost(&p, "d.h").exclusive, 2 * wd);
    assert_eq!(cost(&p, "d.h").expansions, 2);

    let analysis = analyze(&p, &Options::default());
    let b = analysis.header(Path::new("/p/b.h")).unwrap();
    assert_eq!(b.carries, vec![(PathBuf::from("/p/d.h"), wd)]);
}

#[test]
fn partial_guards_are_not_guards() {
    let cases = [
        (
            "code before",
            "int outside;\n#ifndef X_H\n#define X_H\nint x;\n#endif\n",
        ),
        (
            "code after",
            "#ifndef X_H\n#define X_H\nint x;\n#endif\nint outside;\n",
        ),
        (
            "include after",
            "#ifndef X_H\n#define X_H\nint x;\n#endif\n#include \"y.h\"\n",
        ),
        (
            "else branch",
            "#ifndef X_H\n#define X_H\nint x;\n#else\nint y;\n#endif\n",
        ),
        ("wrong macro", "#ifndef X_H\n#define Y_H\nint x;\n#endif\n"),
        ("no define", "#ifndef X_H\nint x;\n#endif\n"),
        (
            "two blocks",
            "#ifndef X_H\n#define X_H\n#endif\n#ifndef Z\nint z;\n#endif\n",
        ),
    ];
    for (what, text) in cases {
        assert_eq!(scan(text.as_bytes()).guard, Guard::None, "{what}");
    }
    let accepted = [
        (
            "comments and blanks outside",
            "// license\n\n/* doc */\n#ifndef X_H\n#define X_H\nint x;\n#endif /* X_H */\n\n",
        ),
        (
            "if !defined()",
            "#if !defined(X_H)\n#define X_H\nint x;\n#endif\n",
        ),
        (
            "if ! defined bare",
            "# if ! defined X_H\n#  define X_H 1\nint x;\n#endif\n",
        ),
        (
            "nested conditionals",
            "#ifndef X_H\n#define X_H\n#ifdef A\nint a;\n#else\nint b;\n#endif\n#endif\n",
        ),
    ];
    for (what, text) in accepted {
        assert_eq!(scan(text.as_bytes()).guard, Guard::Macro, "{what}");
    }

    let d = "int outside;\n#ifndef D_H\n#define D_H\nint d;\n#endif\n";
    let (p, [wa, wb, wc, wd]) = diamond(d);
    assert_eq!(
        p.preprocessed_bytes(0, None),
        wa + wb + wc + 2 * wd,
        "partially guarded D is expanded twice"
    );
    assert_eq!(cost(&p, "d.h").expansions, 2);
}

#[test]
fn conditional_includes_are_followed_and_marked() {
    let a = "#include \"x.h\"\n#ifdef _WIN32\n#include \"win.h\"\n#else\n#include \"posix.h\"\n#endif\n";
    let x = guarded("X", "#include \"inner.h\"\nint x;");
    let p = project(
        &[
            ("a.cpp", a),
            ("x.h", &x),
            ("win.h", "#pragma once\nint w;\n"),
            ("posix.h", "#pragma once\nint px;\n"),
            ("inner.h", "#pragma once\nint i;\n"),
        ],
        &["a.cpp"],
    );
    let inc = &scan(x.as_bytes()).includes;
    assert!(
        !inc[0].conditional,
        "an include inside the guard is not conditional"
    );
    let analysis = analyze(&p, &Options::default());
    assert_eq!(analysis.conditional_includes, 2);
    for (file, conditional) in [
        ("win.h", true),
        ("posix.h", true),
        ("x.h", false),
        ("inner.h", false),
    ] {
        let h = analysis
            .header(Path::new(&format!("/p/{file}")))
            .expect("both branches are taken");
        assert_eq!(h.conditional, conditional, "{file}");
    }
}

#[test]
fn guarded_include_cycles_terminate_and_stay_exact() {
    let x = guarded("X", "#include \"y.h\"\nint x;");
    let y = guarded("Y", "#include \"x.h\"\n#include \"z.h\"\nint y;");
    let z = "#pragma once\n#include \"z.h\"\n#include \"x.h\"\nint z;\n";
    let a = "#include \"x.h\"\n#include \"y.h\"\nint a;\n";
    let p = project(
        &[("a.cpp", a), ("x.h", &x), ("y.h", &y), ("z.h", z)],
        &["a.cpp"],
    );
    let total = weight(a) + weight(&x) + weight(&y) + weight(z);
    assert_eq!(p.preprocessed_bytes(0, None), total);

    let c = tu_costs(&p, 0);
    assert_eq!(c.total, total);
    let file = |n: &str| p.file_id(Path::new(&format!("/p/{n}"))).unwrap();
    // a includes y directly, so x does not dominate y; y alone carries z.
    assert_eq!(c.files[&file("x.h")].exclusive, weight(&x));
    assert_eq!(c.files[&file("y.h")].exclusive, weight(&y) + weight(z));
    assert_eq!(c.files[&file("z.h")].exclusive, weight(z));
}

#[test]
fn unguarded_include_cycles_are_cut_and_reported() {
    let a = "#include \"p.h\"\nint a;\n";
    let ph = "#include \"q.h\"\nint p;\n";
    let qh = "#include \"p.h\"\nint q;\n";
    let p = project(&[("a.cpp", a), ("p.h", ph), ("q.h", qh)], &["a.cpp"]);
    // a -> p -> q -> (p cut)
    assert_eq!(
        p.preprocessed_bytes(0, None),
        weight(a) + weight(ph) + weight(qh)
    );
    let analysis = analyze(&p, &Options::default());
    assert_eq!(analysis.total_bytes, weight(a) + weight(ph) + weight(qh));
    assert_eq!(
        analysis.unguarded_cycles,
        vec![(PathBuf::from("/p/q.h"), PathBuf::from("/p/p.h"))]
    );

    // An unguarded file may legitimately reappear below a guarded one.
    let g = guarded("G", "#include \"u.h\"\nint g;");
    let u = "#include \"g.h\"\nint u;\n";
    let a2 = "#include \"u.h\"\n";
    let p2 = project(&[("a.cpp", a2), ("g.h", &g), ("u.h", u)], &["a.cpp"]);
    assert_eq!(
        p2.preprocessed_bytes(0, None),
        weight(a2) + weight(&g) + 2 * weight(u)
    );
    let c = tu_costs(&p2, 0);
    let uid = p2.file_id(Path::new("/p/u.h")).unwrap();
    assert_eq!(c.files[&uid].expansions, 2);
    assert_eq!(
        c.files[&uid].exclusive,
        weight(&g) + 2 * weight(u),
        "the inner site is nested, not double counted"
    );
    assert!(analyze(&p2, &Options::default())
        .unguarded_cycles
        .is_empty());
}

#[test]
fn self_including_pragma_once_header_terminates() {
    let s = "#pragma once\n#include \"s.h\"\nint s;\n";
    let a = "#include \"s.h\"\n#include \"s.h\"\n";
    let p = project(&[("a.cpp", a), ("s.h", s)], &["a.cpp"]);
    assert_eq!(p.preprocessed_bytes(0, None), weight(a) + weight(s));
    assert_eq!(
        analyze(&p, &Options::default()).total_bytes,
        weight(a) + weight(s)
    );
}
