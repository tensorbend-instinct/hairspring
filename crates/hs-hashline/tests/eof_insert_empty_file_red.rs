//! RED (hostile review, 2026-09-09): `InsertAfter` with the "EOF" sentinel on
//! an EMPTY file inserts after `split_lines`' synthetic trailing-empty line,
//! producing a leading "\n" ("file starts with a blank line"). The synthetic
//! line is a representation artifact, not content: an EOF insert into an
//! empty file must produce exactly the inserted content (+ trailing newline).
//!
//! Falsifier: `new_content` for an EOF insert into "" must equal "x\n", not
//! "\nx".

use hs_hashline::config::HashlineSchemeParams;
use hs_hashline::edit::apply::apply_edits;
use hs_hashline::edit::types::HashlineOp;
use std::path::PathBuf;

#[test]
fn insert_after_eof_on_empty_file_has_no_leading_blank_line() {
    let scheme = HashlineSchemeParams::default().build_scheme().unwrap();
    let ops = vec![HashlineOp::InsertAfter {
        anchor: "EOF".to_owned(),
        content: "x".to_owned(),
    }];
    let r = apply_edits("", &ops, &PathBuf::from("empty.txt"), &*scheme);
    assert!(
        !matches!(r.output, hs_hashline::edit::types::HashlineEditOutput::Error(_)),
        "EOF insert into empty file must succeed"
    );
    let content = r.new_content.expect("new_content on success");
    assert_eq!(
        content, "x\n",
        "EOF insert into empty file must not prepend a blank line"
    );
}
