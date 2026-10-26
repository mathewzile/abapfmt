//! All rules together over the sample files: tokens must survive, and the result must be stable.
use abapfmt_formatter::{format, same_tokens};
use std::collections::BTreeMap;
use std::path::Path;

#[test]
fn samples_survive_the_pipeline() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/samples");
    let none = BTreeMap::new();
    for f in [
        "cleanup-sample.abap",
        "test-1-input.abap",
        "test-2-input.abap",
    ] {
        let src = std::fs::read_to_string(dir.join(f)).unwrap();
        let once = format(&src, &none);
        same_tokens(&src, &once).unwrap_or_else(|e| panic!("{f}: {e}"));
        let twice = format(&once, &none);
        assert_eq!(once, twice, "{f}: not idempotent");
    }
}

#[test]
fn predicate_keywords_are_single_spaced() {
    let src = "CLASS c IMPLEMENTATION.\n  METHOD m.\n    IF a IS  NOT  INITIAL.\n    ENDIF.\n  ENDMETHOD.\nENDCLASS.\n";
    let out = format(src, &BTreeMap::new());
    assert!(out.contains("IF a IS NOT INITIAL."), "{out}");
}
