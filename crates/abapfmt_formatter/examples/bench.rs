//! Throughput of the full pipeline on replicated sample files (identifiers made unique per copy,
//! so that the statement cache of the parser does not see the same statement twice):
//! `cargo run --release -p abapfmt_formatter --example bench [sample]`
use std::collections::BTreeMap;
use std::time::Instant;

use abapfmt_formatter::{Options, format_checked};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "fixtures/samples/cleanup-sample.abap".into());
    let sample = std::fs::read_to_string(&path).expect("sample file");
    let configs = BTreeMap::new();
    let options = Options::default();
    println!("{path}");
    println!("{:>8} {:>10} {:>12}", "lines", "ms", "lines/s");
    for copies in [1, 5, 10, 50, 200] {
        let src: String = (0..copies)
            .map(|i| sample.replace("_", &format!("{i}_")))
            .collect();
        let lines = src.lines().count();
        let runs = if copies <= 50 { 5 } else { 1 };
        let start = Instant::now();
        for _ in 0..runs {
            abapfmt_grammar::clear_memo();
            format_checked(&src, &configs, &options).expect("format");
        }
        let ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(runs);
        println!(
            "{lines:>8} {ms:>10.1} {:>12.0}",
            lines as f64 / (ms / 1000.0)
        );
    }
}
