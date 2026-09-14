use includecost::generate::{generate, GenConfig};
use includecost::{analyze, parse_compile_commands, report, DiskFs, Options, Project};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
includecost: rank C/C++ headers by the preprocessed code their removal would delete

usage:
  includecost analyze <compile_commands.json> [--top N] [--json] [--jobs N]
  includecost generate <dir> [--headers N] [--tus N] [--seed N]

analyze options:
  --top N     show the N most expensive headers (default 20, 0 = all)
  --json      machine-readable output
  --jobs N    worker threads (default: all CPUs)

generate writes a synthetic project and its compile_commands.json into <dir>.
";

enum CliError {
    Usage(String),
    Runtime(String),
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(CliError::Usage(msg)) => {
            eprintln!("error: {msg}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(CliError::Runtime(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::from(1)
        }
    }
}

fn run(args: &[String]) -> Result<(), CliError> {
    match args.first().map(String::as_str) {
        Some("analyze") => cmd_analyze(&args[1..]),
        Some("generate") => cmd_generate(&args[1..]),
        Some("-h" | "--help" | "help") => {
            print!("{USAGE}");
            Ok(())
        }
        Some("-V" | "--version") => {
            println!("includecost {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some(other) => Err(CliError::Usage(format!("unknown command {other:?}"))),
        None => Err(CliError::Usage("missing command".to_owned())),
    }
}

/// Positional arguments and `(flag, value)` pairs; switches get an empty value.
type ParsedArgs = (Vec<String>, Vec<(String, String)>);

fn parse_flags(
    args: &[String],
    valued: &[&str],
    switches: &[&str],
) -> Result<ParsedArgs, CliError> {
    let mut positional = Vec::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if valued.contains(&a.as_str()) {
            let v = args
                .get(i + 1)
                .ok_or_else(|| CliError::Usage(format!("{a} needs a value")))?;
            flags.push((a.clone(), v.clone()));
            i += 2;
        } else if switches.contains(&a.as_str()) {
            flags.push((a.clone(), String::new()));
            i += 1;
        } else if a.starts_with("--") {
            return Err(CliError::Usage(format!("unknown option {a}")));
        } else {
            positional.push(a.clone());
            i += 1;
        }
    }
    Ok((positional, flags))
}

fn number(flag: &str, value: &str) -> Result<usize, CliError> {
    value.parse().map_err(|_| {
        CliError::Usage(format!(
            "{flag} expects a non-negative integer, got {value:?}"
        ))
    })
}

fn cmd_analyze(args: &[String]) -> Result<(), CliError> {
    let (positional, flags) = parse_flags(args, &["--top", "--jobs"], &["--json"])?;
    let [db_path] = positional.as_slice() else {
        return Err(CliError::Usage(
            "analyze takes exactly one compile_commands.json path".to_owned(),
        ));
    };
    let mut top = 20;
    let mut as_json = false;
    let mut opts = Options::default();
    for (flag, value) in &flags {
        match flag.as_str() {
            "--top" => top = number(flag, value)?,
            "--jobs" => opts.threads = number(flag, value)?,
            _ => as_json = true,
        }
    }

    let db_path = PathBuf::from(db_path);
    let text = std::fs::read_to_string(&db_path)
        .map_err(|e| CliError::Runtime(format!("cannot read {}: {e}", db_path.display())))?;
    let base = absolute_parent(&db_path)?;
    let commands = parse_compile_commands(&text, &base)
        .map_err(|e| CliError::Runtime(format!("{}: {e}", db_path.display())))?;
    if commands.is_empty() {
        return Err(CliError::Runtime(format!(
            "{}: no compile commands",
            db_path.display()
        )));
    }

    let project = Project::load(&DiskFs, &commands);
    let analysis = analyze(&project, &opts);
    let out = if as_json {
        report::json(&analysis, top, &base)
    } else {
        report::text(&analysis, top, &base)
    };
    print!("{out}");
    Ok(())
}

fn absolute_parent(path: &Path) -> Result<PathBuf, CliError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    parent
        .canonicalize()
        .map_err(|e| CliError::Runtime(format!("cannot resolve {}: {e}", parent.display())))
}

fn cmd_generate(args: &[String]) -> Result<(), CliError> {
    let (positional, flags) = parse_flags(args, &["--headers", "--tus", "--seed"], &[])?;
    let [dir] = positional.as_slice() else {
        return Err(CliError::Usage(
            "generate takes exactly one output directory".to_owned(),
        ));
    };
    let mut cfg = GenConfig::small(1);
    for (flag, value) in &flags {
        let v = number(flag, value)?;
        match flag.as_str() {
            "--headers" => cfg.headers = v,
            "--tus" => cfg.tus = v,
            _ => cfg.seed = v as u64,
        }
    }
    let project = generate(&cfg);
    let db = project
        .write_to(Path::new(dir))
        .map_err(|e| CliError::Runtime(format!("cannot write {dir}: {e}")))?;
    println!(
        "wrote {} headers, {} translation units, {} include lines\n{}",
        cfg.headers,
        cfg.tus,
        project.include_edges,
        db.display()
    );
    Ok(())
}
