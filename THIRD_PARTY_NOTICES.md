# Third-Party Notices

## crates/hs-applypatch

Vendored source (Apache License 2.0):

- `src/parser.rs`, `src/seek_sequence.rs`, `src/apply.rs`, `src/errors.rs`
  are vendored from **xai-org/grok-build**
  (`crates/codegen/xai-grok-tools/src/implementations/codex/apply_patch/`),
  itself an explicit standalone port of **openai/codex**
  (`codex-rs/apply-patch`). Both projects are Apache License 2.0.

Attribution chain: openai/codex -> xai-org/grok-build -> HAIRSPRING.
The full license text is in `LICENSES/Apache-2.0.txt`.
