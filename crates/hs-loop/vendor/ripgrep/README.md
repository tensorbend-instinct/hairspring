# Packaged ripgrep

These are the same @vscode/ripgrep 1.18.0 platform binaries used by dsh 0.2.1-alpha.1. No system ripgrep or PATH lookup is needed. Each supported target embeds its own binary. Source package URLs, npm SHA-512 integrity and extracted binary SHA-256 are recorded in manifest.json. Tarball integrity was verified before extraction. LICENSE is the unmodified package license.

Supported targets: Linux x64/arm64 and macOS x64/arm64. Unsupported targets fail closed at their first search. The helper is materialized with a random name and owner-only permissions, then removed after each search.

On Linux, exec may briefly return ETXTBSY just after closing a newly materialized executable. Only that pre-spawn errno is retried for at most 3 seconds. A started helper is never restarted, so no output can be duplicated. This is not a model-call timer.
