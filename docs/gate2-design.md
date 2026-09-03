# Gate 2 design: plugin kernel + Rails

Proof gate (spec 10, row 2): add a new tool and a new model by configuration
only - zero harness code changes, zero redeploy.

Sources: DeepSeek Harness (arXiv:2608.25512, everything-is-a-plugin; model
adapters, tools, session state, execution loop all replaceable) and
openJiuwen (arXiv:2608.27969) Rail: rho = (hooks, handler, priority); hooks =
invoke | call(model/tool) | task; priority-ordered, deterministic tie-break;
visibility gating g(rho, subject).

## Decisions

- Plugins are executables speaking newline-delimited JSON over stdio
  (describe / tool.call / model.call / rail.hook). No dylib ABI, crash
  isolation by construction, sandbox-friendly. The harness never links
  against plugin code.
- Config: one TOML file. Adding a capability = adding a config entry + a
  plugin executable. The kernel hot-reloads on config mtime change: the
  harness process is never restarted, which is how "zero redeploy" is proven.
- Rails are plugins too (kind=rail), attached to named hooks with a declared
  priority; equal priorities tie-break by name (deterministic).
- Visibility gating: every config entry carries `subjects`; the kernel
  filters by the calling subject.
- Every tool/model call is recorded to the gate-1 log (kind tool_call /
  model_call, latency_ms, cost_usd_micros). Rails observe via rail.hook;
  hook dispatch is itself NOT logged at this gate (noise), rail failures are
  contained and logged as observation events.

## Cut (belongs to later gates)

No inner loop / context assembly (gate 3), no Goal Mode / budgets / gateway
(gate 4), no schema-constrained model output validation (gate 3), no
mutation of execution state by rails beyond returning observations.
