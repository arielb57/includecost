//! Claims 5 and 6: include search order, compile database parsing, and
//! missing files that must be reported rather than crash.

use includecost::analyze::{analyze, Options};
use includecost::compdb::{parse_compile_commands, SearchPaths};
use includecost::project::Project;
use includecost::resolve::resolve;
use includecost::MemFs;
use std::path::{Path, PathBuf};

fn paths(v: &[&str]) -> Vec<PathBuf> {
    v.iter().map(PathBuf::from).collect()
}

fn search() -> SearchPaths {
    SearchPaths {
        iquote: paths(&["/q1", "/q2"]),
        include: paths(&["/i1", "/i2"]),
        system: paths(&["/s1", "/s2"]),
        after: paths(&["/a1"]),
    }
}

#[test]
fn quoted_includes_search_local_dir_then_iquote_then_i_then_isystem() {
    let order = ["/src/dir", "/q1", "/q2", "/i1", "/i2", "/s1", "/s2", "/a1"];
    let mut fs = MemFs::new();
    for dir in order {
        fs.insert(format!("{dir}/x.h"), "int x;");
    }
    let includer = Path::new("/src/dir/main.cpp");
    for dir in order {
        let found = resolve(&fs, includer, "x.h", false, &search());
        assert_eq!(found, Some(PathBuf::from(format!("{dir}/x.h"))));
        fs.remove(format!("{dir}/x.h"));
    }
    assert_eq!(resolve(&fs, includer, "x.h", false, &search()), None);
}

#[test]
fn angled_includes_skip_local_dir_and_iquote() {
    let mut fs = MemFs::new();
    for dir in ["/src/dir", "/q1", "/q2"] {
        fs.insert(format!("{dir}/x.h"), "");
    }
    let includer = Path::new("/src/dir/main.cpp");
    assert_eq!(resolve(&fs, includer, "x.h", true, &search()), None);
    for dir in ["/i2", "/s1", "/i1"] {
        fs.insert(format!("{dir}/x.h"), "");
    }
    assert_eq!(
        resolve(&fs, includer, "x.h", true, &search()),
        Some(PathBuf::from("/i1/x.h"))
    );
    fs.remove("/i1/x.h");
    assert_eq!(
        resolve(&fs, includer, "x.h", true, &search()),
        Some(PathBuf::from("/i2/x.h"))
    );
    fs.remove("/i2/x.h");
    assert_eq!(
        resolve(&fs, includer, "x.h", true, &search()),
        Some(PathBuf::from("/s1/x.h"))
    );
}

#[test]
fn relative_spellings_are_normalized_and_nested_quotes_use_the_header_dir() {
    let mut fs = MemFs::new();
    fs.insert("/p/lib/detail/impl.h", "#pragma once\nint impl;\n");
    fs.insert(
        "/p/lib/api.h",
        "#pragma once\n#include \"detail/impl.h\"\n#include \"../lib/./detail/impl.h\"\n",
    );
    fs.insert("/p/src/main.cpp", "#include <api.h>\n");
    let db = r#"[{"directory": "/p", "file": "src/main.cpp", "arguments": ["c++", "-I", "lib", "-c", "src/main.cpp"]}]"#;
    let commands = parse_compile_commands(db, Path::new("/")).unwrap();
    let project = Project::load(&fs, &commands);
    assert!(project.unresolved.is_empty());
    let g = project.tu_graph(0);
    assert_eq!(
        g.nodes.len(),
        3,
        "both spellings reach the same impl.h node"
    );
}

#[test]
fn compile_commands_forms_and_flag_spellings() {
    let db = r#"[
      {"directory": "build", "file": "../src/a.cpp",
       "command": "c++ -iquote q -I inc1 -Iinc2 -isystem sys1 -isystemsys2 -idirafter late \"-I/abs path\" -DX -c ../src/a.cpp"},
      {"directory": "/abs", "file": "/abs/b.cpp", "arguments": ["cc", "-I", "x", "-I"]}
    ]"#;
    let cmds = parse_compile_commands(db, Path::new("/root")).unwrap();
    assert_eq!(cmds[0].directory, PathBuf::from("/root/build"));
    assert_eq!(cmds[0].file, PathBuf::from("/root/src/a.cpp"));
    assert_eq!(
        cmds[0].search,
        SearchPaths {
            iquote: paths(&["/root/build/q"]),
            include: paths(&["/root/build/inc1", "/root/build/inc2", "/abs path"]),
            system: paths(&["/root/build/sys1", "/root/build/sys2"]),
            after: paths(&["/root/build/late"]),
        }
    );
    assert_eq!(
        cmds[1].search.include,
        paths(&["/abs/x"]),
        "a dangling -I is ignored"
    );
}

#[test]
fn malformed_compile_commands_are_errors_not_panics() {
    for bad in [
        "{}",
        "[{\"file\": \"a.cpp\", \"command\": \"cc\"}]",
        "[{\"directory\": \"/\", \"command\": \"cc\"}]",
        "[{\"directory\": \"/\", \"file\": \"a.cpp\"}]",
        "[{\"directory\": \"/\", \"file\": \"a.cpp\", \"arguments\": [1]}]",
        "[{\"directory\": \"/\", \"file\": \"a.cpp\", \"arguments\": \"cc\"}]",
        "not json",
    ] {
        assert!(
            parse_compile_commands(bad, Path::new("/")).is_err(),
            "accepted {bad}"
        );
    }
}

#[test]
fn missing_headers_and_sources_are_reported_without_crashing() {
    let mut fs = MemFs::new();
    fs.insert(
        "/p/a.cpp",
        "#include \"real.h\"\n#include \"nope.h\"\n\n#include <vector>\n#include CONFIG_H\n#include \"\"\n",
    );
    fs.insert(
        "/p/real.h",
        "#pragma once\n#include \"deeper_missing.h\"\nint r;\n",
    );
    fs.insert("/p/b.cpp", "#include \"nope.h\"\n#include \"real.h\"\n");
    let db = r#"[
      {"directory": "/p", "file": "a.cpp", "command": "cc -c a.cpp"},
      {"directory": "/p", "file": "gone.cpp", "command": "cc -c gone.cpp"},
      {"directory": "/p", "file": "b.cpp", "command": "cc -c b.cpp"}
    ]"#;
    let commands = parse_compile_commands(db, Path::new("/")).unwrap();
    let project = Project::load(&fs, &commands);
    let analysis = analyze(&project, &Options::default());

    assert_eq!(analysis.translation_units, 2);
    assert_eq!(analysis.missing_sources, paths(&["/p/gone.cpp"]));
    let mut unresolved: Vec<(String, u32, String, bool)> = analysis
        .unresolved
        .iter()
        .map(|(p, l, s, a)| (p.display().to_string(), *l, s.clone(), *a))
        .collect();
    unresolved.sort();
    assert_eq!(
        unresolved,
        vec![
            ("/p/a.cpp".to_owned(), 2, "nope.h".to_owned(), false),
            ("/p/a.cpp".to_owned(), 4, "vector".to_owned(), true),
            ("/p/b.cpp".to_owned(), 1, "nope.h".to_owned(), false),
            (
                "/p/real.h".to_owned(),
                2,
                "deeper_missing.h".to_owned(),
                false
            ),
        ]
    );
    // `#include CONFIG_H` and the empty `#include ""` cannot be resolved lexically.
    assert_eq!(analysis.computed_includes, 2);
    let real = analysis.header(Path::new("/p/real.h")).unwrap();
    assert_eq!(real.tus, 2);
    assert_eq!(analysis.headers.len(), 1);
}

#[test]
fn unreadable_database_entries_leave_an_empty_but_valid_analysis() {
    let fs = MemFs::new();
    let db = r#"[{"directory": "/p", "file": "a.cpp", "command": "cc a.cpp"}]"#;
    let commands = parse_compile_commands(db, Path::new("/")).unwrap();
    let analysis = analyze(&Project::load(&fs, &commands), &Options::default());
    assert_eq!(analysis.translation_units, 0);
    assert!(analysis.headers.is_empty());
    assert_eq!(analysis.total_bytes, 0);
}
