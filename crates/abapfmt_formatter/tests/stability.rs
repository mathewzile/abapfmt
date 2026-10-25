//! Formatting the output of the formatter again changes nothing, also with a narrow `lineWidth`,
//! and empty lines at the end of a CDS source are removed.
use std::collections::BTreeMap;

use abapfmt_formatter::{Options, format_checked};

#[test]
fn narrow_line_width_is_stable() {
    let sample = std::fs::read_to_string("../../fixtures/samples/cleanup-sample.abap").unwrap();
    let configs = BTreeMap::new();
    for width in [60, 80, 100, 120] {
        let options = Options {
            line_width: Some(width),
        };
        let once = format_checked(&sample, &configs, &options).unwrap();
        let twice = format_checked(&once, &configs, &options).unwrap();
        assert_eq!(once, twice, "line width {width}");
    }
}

#[test]
fn empty_lines_at_document_end_are_removed() {
    let src = "define view entity A as select from B\n{\n  key B.x\n}\n\n\n";
    let out = format_checked(src, &BTreeMap::new(), &Options::default()).unwrap();
    assert!(out.ends_with("}\n") && !out.ends_with("\n\n"), "{out:?}");
}
