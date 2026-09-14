//! Reading `compile_commands.json` and extracting header search paths.

use crate::fs::normalize;
use crate::json::{self, Value};
use std::path::{Path, PathBuf};

/// Header search directories of one compile command, in command-line order
/// within each class.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct SearchPaths {
    pub iquote: Vec<PathBuf>,
    pub include: Vec<PathBuf>,
    pub system: Vec<PathBuf>,
    pub after: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileCommand {
    pub directory: PathBuf,
    pub file: PathBuf,
    pub search: SearchPaths,
}

/// Parses a compilation database. Relative `directory` entries are taken
/// relative to `base_dir`, normally the directory holding the JSON file.
pub fn parse_compile_commands(text: &str, base_dir: &Path) -> Result<Vec<CompileCommand>, String> {
    let root = json::parse(text)?;
    let entries = root
        .as_array()
        .ok_or("compile_commands.json must contain a JSON array")?;
    let mut out = Vec::with_capacity(entries.len());
    for (i, entry) in entries.iter().enumerate() {
        let field = |name: &str| entry.get(name).and_then(Value::as_str);
        let directory =
            field("directory").ok_or_else(|| format!("entry {i}: missing \"directory\""))?;
        let file = field("file").ok_or_else(|| format!("entry {i}: missing \"file\""))?;
        let args: Vec<String> = if let Some(arguments) = entry.get("arguments") {
            arguments
                .as_array()
                .ok_or_else(|| format!("entry {i}: \"arguments\" must be an array"))?
                .iter()
                .map(|a| {
                    a.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| format!("entry {i}: \"arguments\" must contain strings"))
                })
                .collect::<Result<_, _>>()?
        } else if let Some(command) = field("command") {
            split_command(command)
        } else {
            return Err(format!("entry {i}: needs \"arguments\" or \"command\""));
        };
        let directory = normalize(&base_dir.join(directory));
        let file = normalize(&directory.join(file));
        let search = search_paths_from_args(&args, &directory);
        out.push(CompileCommand {
            directory,
            file,
            search,
        });
    }
    Ok(out)
}

/// POSIX-shell-style word splitting, which is how `command` strings are
/// specified to be interpreted.
pub fn split_command(command: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = command.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    cur.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' if matches!(chars.peek(), Some('"' | '\\' | '$' | '`')) => {
                            cur.push(chars.next().unwrap_or('\\'));
                        }
                        q => cur.push(q),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    words
}

pub fn search_paths_from_args(args: &[String], directory: &Path) -> SearchPaths {
    let mut sp = SearchPaths::default();
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].as_str();
        for (flag, class) in [
            ("-iquote", 0),
            ("-isystem", 2),
            ("-idirafter", 3),
            ("-I", 1),
        ] {
            let Some(rest) = arg.strip_prefix(flag) else {
                continue;
            };
            let value = if rest.is_empty() {
                i += 1;
                match args.get(i) {
                    Some(v) => v.as_str(),
                    None => break,
                }
            } else {
                rest
            };
            let dir = normalize(&directory.join(value));
            match class {
                0 => sp.iquote.push(dir),
                1 => sp.include.push(dir),
                2 => sp.system.push(dir),
                _ => sp.after.push(dir),
            }
            break;
        }
        i += 1;
    }
    sp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_like_a_shell() {
        let words = split_command(r#"c++ -I"dir with space" -I 'q d' -DX=\"y\" a\ b.cpp"#);
        assert_eq!(
            words,
            [
                "c++",
                "-Idir with space",
                "-I",
                "q d",
                "-DX=\"y\"",
                "a b.cpp"
            ]
        );
    }
}
