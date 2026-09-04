# MCP Adapter Gate (scoped 2026-09-04)

Status: scoped, not started. Lands after the A/B/C benchmark (user expectation).

## Decision
HAIRSPRING acts as an MCP **client** for workspace-local tools, via the official
Rust SDK `rmcp` (github.com/modelcontextprotocol/rust-sdk, client feature,
`transport-child-process`). The adapter runs as its own plugin process speaking
the kernel's existing stdio JSON protocol - the kernel stays untouched.

## Two MCP surfaces (do not conflate)
1. **Client-side MCP (this gate).** Kernel spawns the adapter; adapter spawns an
   MCP server per mission (e.g. an off-the-shelf filesystem server); tools
   execute on-box. Sandboxing holds (workspace-root prefix check at the adapter
   + server's own allowed-dirs) and event-log auditing holds automatically
   (every call routes through kernel.call_tool -> ToolCall events).
2. **Server-side MCP (z.ai feature, docs.z.ai/guides/capabilities/mcp-call).**
   MCP servers declared inside chat/completions; z.ai's backend executes them.
   Cannot reach our workspace; our auditing does not apply. Optional later
   config flag for REMOTE tools only (e.g. webSearchPrime). Never for repo tools.

## Work items (est. 1.5-2 days TDD)
1. `[[mcp_servers]]` config schema: name, command, args, allowed roots.
2. Adapter plugin `hs-plugin-mcpbridge`: per-mission server lifecycle
   (spawn at mission start, kill at end), rmcp client handshake, list_tools ->
   namespaced kernel registry entries (`mcp.<server>.<tool>`), call_tool
   JSON pass-through, error mapping.
3. Path-arg prefix enforcement at the adapter (defense in depth with the
   server's own allowed-dirs).
4. Async friction: rmcp is tokio; the bridge owns a small runtime and speaks
   sync stdio JSON lines to the kernel. No kernel changes.
5. Contract tests: fixture MCP server (rmcp server impl in-tree), red-first:
   discovery, namespacing, sandbox rejection, audit events present.
6. repo.read/repo.search remain the zero-dependency baseline; MCP is an
   alternative tool surface behind config, not a replacement.
