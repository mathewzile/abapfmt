use abapfmt_syntax::{Options, is_ddl_source, parse, parse_ddl};

fn main() {
    let path = std::env::args().nth(1).unwrap();
    let src = std::fs::read_to_string(path).unwrap();
    if is_ddl_source(&src) {
        let s = parse_ddl(&src).expect("ddl");
        let d = s.ddl.as_ref().unwrap();
        for (k, c) in s.commands.iter().enumerate() {
            let text: Vec<&str> = (c.first..=c.last).map(|i| s.text(i)).collect();
            println!(
                "CMD{k} parent={:?} opens={} closes={} {}",
                d.cmd_parent[k],
                d.cmd_opens[k],
                d.cmd_closes[k],
                text.join(" ")
            );
        }
        for i in 0..s.tokens.len() {
            println!("{i:3} {:?} {:?}", s.text(i), s.types[i]);
        }
        return;
    }
    let s = parse(&src, Options::default()).unwrap();
    for c in &s.commands {
        println!(
            "CMD {:?} first={} last={} full={}",
            c.statement, c.first, c.last, c.fully_matched
        );
    }
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
