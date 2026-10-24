use abapfmt_syntax::{Options, parse};

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let src = std::fs::read_to_string(path).unwrap();
    let s = parse(&src, Options::default()).unwrap();
    for i in 0..s.tokens.len() {
        println!(
            "{i:3} {:?} {:?} lines={} sp={} br={:?} par={:?}",
            s.text(i),
            s.types[i],
            s.tokens[i].lines_before,
            s.tokens[i].spaces_before,
            s.bracket_match[i],
            s.parent[i]
        );
    }
}
