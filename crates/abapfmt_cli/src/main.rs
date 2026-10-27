//! The `abapfmt` command line.
#![forbid(unsafe_code)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};

use abapfmt_config::{Config, Loaded};
use abapfmt_formatter::{Skipped, format_safe};

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
        --print-config          Print the complete configuration (all rules with all options):
                                the defaults, or the settings of --config FILE; this also converts
                                an abap-cleaner profile (.cfj) to abapfmt.json
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
    print_config: bool,
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
            "--print-config" => o.print_config = true,
            "--stdin-filepath" => {
                o.stdin_filepath =
                    Some(args.next().ok_or("--stdin-filepath needs a value")?.into());
            }
            "--config" => o.config = Some(args.next().ok_or("--config needs a value")?.into()),
            s if s.starts_with('-') => return Err(format!("unknown option {s}")),
            _ => o.paths.push(arg.into()),
        }
    }
    if o.print_config {
        return Ok(Some(o));
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
        let i = self.index_for_file(file)?;
        Ok(&self.cache[i].1)
    }

    /// The index in `cache` of the configuration of `file`.
    fn index_for_file(&mut self, file: &Path) -> Result<usize, String> {
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
            return Ok(i);
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
        Ok(self.cache.len() - 1)
    }
}

/// What formatting one file produced; reported after all files are done, in input order.
#[derive(Default)]
struct Outcome {
    /// Warnings and errors for stderr.
    messages: Vec<String>,
    /// For `--check`: the file is not formatted.
    unformatted: bool,
    /// The file could not be formatted, or only with skipped rules.
    failed: bool,
    /// Without `--write` or `--check`: the formatted code.
    output: Option<String>,
}

fn process(file: &Path, config: &Config, options: &Options) -> Outcome {
    let mut outcome = Outcome::default();
    let name = file.display();
    let src = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            outcome.messages.push(format!("error: {name}: {e}"));
            outcome.failed = true;
            return outcome;
        }
    };
    let formatted = match format_safe(&src, &config.rules, &config.options()) {
        Ok(f) => f,
        Err(e) => {
            outcome.messages.push(format!("error: {name}: {e}"));
            outcome.failed = true;
            return outcome;
        }
    };
    outcome.messages = skipped_messages(&name.to_string(), &formatted.skipped);
    outcome.failed = !formatted.skipped.is_empty();
    let out = formatted.text;
    if options.check {
        outcome.unformatted = out != src;
    } else if options.write {
        if out != src
            && let Err(e) = std::fs::write(file, &out)
        {
            outcome.messages.push(format!("error: {name}: {e}"));
            outcome.failed = true;
        }
    } else {
        outcome.output = Some(out);
    }
    outcome
}

/// Formats the files on all cores. The results are in the order of `jobs`.
fn process_all(jobs: &[(&Path, &Config)], options: &Options) -> Vec<Outcome> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, jobs.len().max(1));
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, Outcome)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some((file, config)) = jobs.get(i) else {
                            break mine;
                        };
                        mine.push((i, process(file, config, options)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("worker panicked"))
            .collect()
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, o)| o).collect()
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

/// A warning for each rule that was switched off to keep the code unchanged. A file with such
/// warnings counts as an error, although its safe result is used.
fn skipped_messages(name: &str, skipped: &[Skipped]) -> Vec<String> {
    skipped
        .iter()
        .map(|s| {
            format!(
                "warning: {name}: rule {} skipped, its result would have changed the code ({})",
                s.rule, s.error
            )
        })
        .collect()
}

fn run() -> Result<ExitCode, String> {
    if std::env::args().nth(1).as_deref() == Some("lsp") {
        abapfmt_lsp::run()?;
        return Ok(ExitCode::SUCCESS);
    }
    let Some(options) = parse_args(std::env::args().skip(1))? else {
        return Ok(ExitCode::SUCCESS);
    };
    if options.print_config {
        let loaded = match &options.config {
            Some(path) => abapfmt_config::load_file(path)?,
            None => abapfmt_config::Loaded::default(),
        };
        for w in &loaded.warnings {
            eprintln!("warning: {w}");
        }
        print!("{}", loaded.config.to_json());
        return Ok(ExitCode::SUCCESS);
    }
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
        let out = {
            let config = configs.for_file(&file)?;
            format_safe(&src, &config.rules, &config.options())
        }
        .map_err(|e| format!("stdin: {e}"))?;
        std::io::stdout()
            .write_all(out.text.as_bytes())
            .map_err(|e| e.to_string())?;
        for m in skipped_messages("stdin", &out.skipped) {
            eprintln!("{m}");
        }
        return Ok(if out.skipped.is_empty() {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(2)
        });
    }

    let mut files = Vec::new();
    for p in &options.paths {
        collect_files(p, &mut files)?;
    }
    if !options.write && !options.check && files.len() != 1 {
        return Err("without --write or --check, exactly one file can be formatted".into());
    }

    // configurations are resolved first, in order, so that each is loaded (and warned about) once
    let mut indices = Vec::with_capacity(files.len());
    for file in &files {
        indices.push(configs.index_for_file(file)?);
    }
    let jobs: Vec<(&Path, &Config)> = files
        .iter()
        .zip(indices)
        .map(|(f, i)| (f.as_path(), &configs.cache[i].1))
        .collect();

    let (mut unformatted, mut failed) = (0, 0);
    for ((file, _), outcome) in jobs.iter().zip(process_all(&jobs, &options)) {
        for m in &outcome.messages {
            eprintln!("{m}");
        }
        if outcome.failed {
            failed += 1;
        }
        if outcome.unformatted {
            println!("{}", file.display());
            unformatted += 1;
        }
        if let Some(out) = outcome.output {
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
