//! Prints the imported options of rules: `cargo run --example show -- profile.cfj RULE...`
fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("profile");
    let loaded = abapfmt_config::load_file(std::path::Path::new(&path)).expect("load");
    for id in args {
        println!("{id}: {:?}", loaded.config.rules.get(&id));
    }
}
