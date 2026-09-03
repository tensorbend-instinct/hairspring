# Gate 6 design: shared world + executable inheritance

Spec row (section 10): "Two agents on disjoint schedules: agent B reuses
agent A's installed controller by world observation with zero messages;
uninstall A entirely and the controller keeps acting (SwarmWorld's own
assay shape)."
Falsifiable: any message passing between A and B, B failing to reuse, or
the controller stopping when A is uninstalled all fail the gate.

## World service (crate hs-world)

The shared deterministic environment. Core rule (proposal-consequence
separation, PROVEN in SwarmWorld): agents only ever write PROPOSAL events;
the world service alone validates and writes CONSEQUENCE events. The
agent's description of value is never the measurement of value.

Artifact record (spec section, schema is our proposal):
```
Artifact { artifact_id, version, kind: file|program|controller|note|skill,
           content_hash, world_path, author_stream, created_event,
           parent_version, status: proposed|validated|installed|retired,
           install_event }
Controller { artifact_ref, tick_schedule, last_result,
             runs_without_model_call: true }
```

## Gate scope (cut list applies)

- World state = a directory tree (world_path space) + an artifact registry
  stored as events on a dedicated world stream in the same log root.
- API: propose(artifact) -> validates (schema + content hash + status
  transitions) -> consequence event; install(artifact_id) for controllers;
  uninstall(author_stream) retires THAT AGENT's artifacts only;
  observe(world_path) reads current world state (this is the zero-message
  channel: B reads the world, never messages A).
- Controller at gate 6: an installed program with a tick schedule. A world
  tick runner executes installed controllers between agent decisions
  (runs_without_model_call: true). Controllers read/write world_paths via
  proposal events only.
- Executable inheritance: artifacts carry parent_version for fork lineage;
  a controller installed from an artifact KEEPS RUNNING off the artifact
  record (content-addressed) even when its author stream is uninstalled.

## Proof test shape (tests/gate6_proof.rs)

1. Agent A (stream A, schedule X): proposes + validates + installs a
   controller C (a trivial program: on each tick, append a counter to a
   world file via proposal events).
2. Ticks run; C's consequences land in the world.
3. Agent B (stream B, disjoint schedule, NO gateway messages, NO shared
   memory beyond the world): observes world_path, finds C's record,
   reuses C's output artifact in its own task.
4. Uninstall A entirely (retire A's artifacts, A's stream goes silent):
   C keeps acting on subsequent ticks (consequence events keep landing,
   authored by the world service off the installed artifact).
5. THE GATE: assert zero A<->B message events exist anywhere in the log;
   B's stream contains an observation of C's artifact; post-uninstall
   ticks still produce C consequences; all chains verify.
6. Adversarial: a proposal with a bad content_hash or illegal status
   transition is rejected by the world service, and the rejection is
   itself a consequence event.

## Cut for gate 6

No quarantine forks (gate 8), no scorer pins (gate 7), no skills library,
no snapshot scheduler (gate 1 tiers already cover resume). Controller
sandboxing = the existing plugin subprocess model.
