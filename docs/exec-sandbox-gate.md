# Exec Sandbox Gate (scoped 2026-09-04, Eric's directive)

Status: scoped, not started. Queue: after repo.exec seam test + live smoke
mission (verification of the interim allowlist version), alongside/before
provider-config collapse. Replaces the allowlist when landed.

## Eric's directive
repo.exec should run ANY command the model thinks it needs before
submission, not an allowlist. Agreed path (parent 13:59): per-mission
sandbox so an open shell is safe by construction; allowlist stays as the
interim guardrail until this lands.

## Why the allowlist exists at all
Today's repo.exec runs the command as the harness user with the mission's
full environment and filesystem. An open shell in that shape could read the
GLM key file (~/.keys), mutate the live workspace or repo checkout, or
exfiltrate over the network. The allowlist is a policy crutch; the sandbox
removes the need for it.

## Design: bwrap per-exec sandbox (verified available on-box)
`bwrap` + unprivileged user namespaces both confirmed working (2026-09-04).
Per repo.exec call:

1. Scratch: current answer patch applied to a scratch git worktree
   (unchanged from the allowlist version).
2. Isolation: `bwrap --unshare-all --die-with-parent`
   - mount ns: ONLY the scratch worktree bound (rw); a fresh tmpfs /tmp;
     nothing else of the host fs exists - no ~/.keys, no live ws, no repo.
   - net ns: empty - network OFF by construction (SWE-bench tasks must not
     phone home; the model's provider calls never transit repo.exec).
   - pid/ipc/uts ns: private; --die-with-parent kills the tree on harness
     exit (supervisor relaunch safety).
3. Env scrub: child gets a minimal allowlist env (PATH, HOME=/tmp, LANG,
   TERM) - never the mission env; no HS_* vars, no key paths.
4. Resources: existing hard timeout (kill at limit) + prlimit wrapper
   (AS 4GB, NPROC 256, FSIZE 256MB, NOFILE 1024) so a runaway build can't
   starve the box; stdout/stderr tail-capped 8KB (existing).
5. Audit: unchanged - every call is a kernel ToolCall event on the stream.

## TDD work items (est. ~1 day)
1. RED contract tests (extend ops-level repexec suite):
   - fs isolation: child cannot read the answer file's repo path outside
     the scratch, cannot see ~/.keys (assert read fails), cannot see the
     live workspace.
   - env scrub: `env` output in child contains no HS_* / no *_KEY*.
   - network off: connect to 127.0.0.1:8787 (the relay!) fails inside the
     sandbox - proves net ns is empty, not just unreachable-by-policy.
   - open shell: an arbitrary non-allowlisted command (e.g. `echo hi |
     rev`) now RUNS and returns output (the old allowlist rejection test
     is inverted behind the sandbox config flag).
   - resource cap: a memory-bomber (`tail /dev/zero`) is killed by rlimit,
     reported as such; timeout path unchanged.
   - die-with-parent: kill the harness mid-exec, assert no orphan remains.
2. Implementation: repexec::run gains sandbox mode (bwrap command line
   builder, pure function, unit-tested string); config flag in
   hairspring.toml ([exec] sandbox = true, allowlist fallback); plugin
   passes it through.
3. Seam test: scripted benchmodel mission where the model runs an
   off-allowlist command successfully through the real kernel path.
4. Live GLM smoke mission re-run of one arm with sandbox on: confirm the
   model uses free-form exec (pip, grep, pytest) and nothing escapes.
5. Docs: README ops section + this doc's status flip.

## Non-goals
- No network policy beyond off/on (a per-mission allow-net flag can come
  later if a task genuinely needs pip installs).
- No VM-level isolation (microVM); namespaces are sufficient for a
  single-tenant mission box.
