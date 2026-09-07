// Vendored from xai-org/grok-build (Apache-2.0), crates/codegen/xai-grok-tools/src/implementations/grok_build_hashline/read_file.rs
// See THIRD_PARTY_NOTICES.md.

use crate::anchor::split_lines;
use crate::scheme::AnchorScheme;

/// Format file content lines with anchor annotations.
///
/// Each line is formatted as `LINE:ANCHOR→CONTENT`.
/// `ANCHOR` is the scheme-generated anchor for that line.
///
/// Returns `(hashline_content, raw_output)`.
pub fn format_hashline_content(
    file_content: &str,
    offset: Option<usize>,
    limit: Option<usize>,
    scheme: &dyn AnchorScheme,
) -> (String, String) {
    use std::fmt::Write as _;

    let all_lines = split_lines(file_content);
    let anchors = scheme.generate_anchors(&all_lines);

    let skip = offset.unwrap_or(1).saturating_sub(1);
    let take = limit.unwrap_or(usize::MAX);

    let mut output = String::new();
    let mut raw_output = String::new();
    let mut first_line: Option<usize> = None;

    for (i, line) in all_lines.iter().enumerate().skip(skip).take(take) {
        let line_num = i + 1; // 1-based

        if first_line.is_none() {
            first_line = Some(line_num);
        } else {
            output.push('\n');
            raw_output.push('\n');
        }

        // Build the anchor suffix: "local" or "local:context" (without line number,
        // since we format the line number separately with right-alignment).
        let anchor_suffix = match &anchors[i].context {
            Some(ctx) => format!("{}:{ctx}", anchors[i].local),
            None => anchors[i].local.clone(),
        };

        // Format: "LINE:LOCAL:CONTEXT→CONTENT" (or "LINE:LOCAL→CONTENT" for A)
        _ = write!(&mut output, "{line_num}:{anchor_suffix}→{line}").ok();
        raw_output.push_str(line);
    }

    (output, raw_output)
}
