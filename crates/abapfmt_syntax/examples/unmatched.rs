//! Lists commands the grammar did not match: `cargo run --example unmatched -- files...`
use abapfmt_syntax::{Options, parse};

fn main() {
    let mut shown = std::collections::BTreeSet::new();
    let (mut total, mut bad) = (0, 0);
    for path in std::env::args().skip(1) {
        let Ok(src) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(syn) = parse(&src, Options::default()) else {
            continue;
        };
        for c in &syn.commands {
            if c.is_comment_only {
                continue;
            }
            total += 1;
            if !c.fully_matched {
                bad += 1;
                let text: Vec<&str> = (c.first..=c.last).map(|i| syn.text(i)).collect();
                let line = text.join(" ");
                if shown.insert(line.clone()) && shown.len() <= 40 {
                    println!("{}", &line[..line.len().min(150)]);
                }
            }
        }
    }
    println!("{bad} of {total} commands not fully matched");
}
