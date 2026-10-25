//! The `abapfmt` command line.
#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use abapfmt_config::{Config, Loaded};
use abapfmt_formatter::format_checked;

const HELP: &str = "\
abapfmt - an ABAP and CDS formatter

USAGE:
    abapfmt [OPTIONS] <PATH>...
    abapfmt [OPTIONS] --stdin
    abapfmt lsp

Directories are searched for ABAP and CDS (DDL/DCL) files. Without --write or --check, the
formatted code of a single file is printed.

The lsp subcommand runs the language server (document and range formatting) on stdio.

OPTIONS:
    -w, --write                 Rewrite files in place
        --check                 List files that are not formatted; exit with 1 if there are any
        --stdin                 Read the code from stdin and print the result
        --stdin-filepath <P>    Path of the stdin code, for the configuration lookup
        --config <FILE>         Use this abapfmt.json or abap-cleaner profile (.cfj)
        --no-config             Do not look for abapfmt.json
    -h, --help                  Print this help
    -V, --version               Print the version
";

const EXTENSIONS: &[&str] = &[
    "abap", "acds", "asddls", "asdcls", "ddls", "dcls", "ddl", "dcl",
];

#[derive(Default)]
struct Options {
    write: bool,
    check: bool,
    stdin: bool,
    stdin_filepath: Option<PathBuf>,
    config: Option<PathBuf>,
    no_config: bool,
    paths: Vec<PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<Options>, String> {
    let mut o = Options::default();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("abapfmt {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "-w" | "--write" => o.write = true,
            "--check" => o.check = true,
            "--stdin" => o.stdin = true,
            "--no-config" => o.no_config = true,
            "--stdin-filepath" => {
                o.stdin_filepath =
                    Some(args.next().ok_or("--stdin-filepath needs a value")?.into());
            }
            "--config" => o.config = Some(args.next().ok_or("--config needs a value")?.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            _ => o.paths.push(arg.into()),
        }
    }
    if o.stdin && (o.write || o.check || !o.paths.is_empty()) {
        return Err("--stdin cannot be combined with paths, --write or --check".into());
    }
    if o.write && o.check {
        return Err("--write and --check cannot be combined".into());
    }
    if !o.stdin && o.paths.is_empty() {
        return Err("no input; pass paths or --stdin (see --help)".into());
    }
    Ok(Some(o))
}

/// Resolves configuration files, loading each only once.
struct Configs<'a> {
    options: &'a Options,
    cache: Vec<(Option<PathBuf>, Config)>,
}

impl Configs<'_> {
    fn for_file(&mut self, file: &Path) -> Result<&Config, String> {
        let path = if let Some(p) = &self.options.config {
            Some(p.clone())
        } else if self.options.no_config {
            None
        } else {
            let dir = std::path::absolute(file)
                .ok()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_default();
            abapfmt_config::find_config(&dir)
        };
        if let Some(i) = self.cache.iter().position(|(p, _)| *p == path) {
            return Ok(&self.cache[i].1);
        }
        let config = match &path {
            Some(p) => {
                let Loaded { config, warnings } = abapfmt_config::load_file(p)?;
                for w in warnings {
                    eprintln!("warning: {}: {w}", p.display());
                }
                config
            }
            None => Config::default(),
        };
        self.cache.push((path, config));
        Ok(&self.cache.last().expect("just pushed").1)
    }
}

fn collect_files(path: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    if path.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .filter_map(Result::ok)
            .map(|e| e.path())
            .collect();
        entries.sort();
        for p in entries {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if p.is_dir() {
                if !(name.starts_with('.') || name == "node_modules" || name == "target") {
                    collect_files(&p, out)?;
                }
            } else if has_known_extension(&p) {
                out.push(p);
            }
        }
        Ok(())
    } else if path.is_file() {
        out.push(path.to_path_buf());
        Ok(())
    } else {
        Err(format!("{}: no such file or directory", path.display()))
    }
}

fn has_known_extension(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

fn run() -> Result<ExitCode, String> {
    if std::env::args().nth(1).as_deref() == Some("lsp") {
        abapfmt_lsp::run()?;
        return Ok(ExitCode::SUCCESS);
    }
    let Some(options) = parse_args(std::env::args().skip(1))? else {
        return Ok(ExitCode::SUCCESS);
    };
    let mut configs = Configs {
        options: &options,
        cache: Vec::new(),
    };

    if options.stdin {
        let mut src = String::new();
        std::io::stdin()
            .read_to_string(&mut src)
            .map_err(|e| format!("stdin: {e}"))?;
        let file = options.stdin_filepath.clone().unwrap_or_default();
        let out = format_checked(&src, &configs.for_file(&file)?.rules)
            .map_err(|e| format!("stdin: {e}"))?;
        std::io::stdout()
            .write_all(out.as_bytes())
            .map_err(|e| e.to_string())?;
        return Ok(ExitCode::SUCCESS);
    }

    let mut files = Vec::new();
    for p in &options.paths {
        collect_files(p, &mut files)?;
    }
    if !options.write && !options.check && files.len() != 1 {
        return Err("without --write or --check, exactly one file can be formatted".into());
    }

    let (mut unformatted, mut failed) = (0, 0);
    for file in &files {
        let src = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("error: {}: {e}", file.display());
                failed += 1;
                continue;
            }
        };
        let config = configs.for_file(file)?;
        let out = match format_checked(&src, &config.rules) {
            Ok(o) => o,
            Err(e) => {
                eprintln!("error: {}: {e}", file.display());
                failed += 1;
                continue;
            }
        };
        if options.check {
            if out != src {
                println!("{}", file.display());
                unformatted += 1;
            }
        } else if options.write {
            if out != src {
                std::fs::write(file, &out).map_err(|e| format!("{}: {e}", file.display()))?;
            }
        } else {
            std::io::stdout()
                .write_all(out.as_bytes())
                .map_err(|e| e.to_string())?;
        }
    }
    Ok(if failed > 0 {
        ExitCode::from(2)
    } else if unformatted > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(2)
        }
    }
}
