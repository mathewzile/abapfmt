//! Per-rule time on a replicated sample: `cargo run --release -p abapfmt_formatter --example prof [copies]`
use std::time::Instant;

use abapfmt_formatter::{rules, run_rule};

fn main() {
    let copies: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(5);
    let mut text = std::fs::read_to_string("fixtures/samples/cleanup-sample.abap")
        .unwrap()
        .repeat(copies);
    let config = Default::default();
    let mut times = Vec::new();
    let mut changed = 0;
    for (id, _) in rules() {
        let start = Instant::now();
        if let Some(out) = run_rule(id, &text, &config) {
            if out != text {
                changed += 1;
            }
            text = out;
        }
        times.push((start.elapsed().as_secs_f64() * 1000.0, *id));
    }
    let total: f64 = times.iter().map(|t| t.0).sum();
    times.sort_by(|a, b| b.0.total_cmp(&a.0));
    println!(
        "{} rules, {changed} changed the text, total {total:.0} ms",
        times.len()
    );
    println!(
        "min {:.1} ms, median {:.1} ms",
        times.last().unwrap().0,
        times[times.len() / 2].0
    );
    for (ms, id) in times.iter().take(8) {
        println!("{ms:>8.1} {id}");
    }
}
