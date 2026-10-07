# Native dsh bash/job rollout, local work

Exact captured input tool contracts: bash, job_list, job_output, job_kill.

Implemented and focused tested:
- fresh confined shell, session-owned jobs, timeout promotion instead of kill;
- distinct stdout/stderr, bounded UTF8 tails, spill files, incremental output;
- explicit TERM/KILL cancellation, teardown cancellation, owner isolation;
- completion notices at active mission step boundaries, suppressed when awaited or cancelled;
- native-history model rendering and final-answer uncollected-job gate;
- confined Rust supervisor distinguishes numeric exit143 from SIGTERM;
- supervisor frames cannot be spoofed by child stdout; helper included in install/CI copy list;
- spill-directory symlink rejection, creation-before-launch, output-worker failure reporting.

Not exact-complete:
- idle session autonomous completion followup;
- sandbox escalation approved through the user channel (currently reject wider requests);
- environment profile/plugin contributor registry;
- durable resumed job records (source dsh registry is in-memory; compare owner-disposal semantics rather than assume persistence);
- sandbox denial/result metadata and full model-result budget behavior;
- teardown synchronously awaiting producers;
- I/O failure unit injection currently exercises a legacy pump helper, not the production framed-drain branch; requires replacement before claiming I/O trap proof.

Observed RED/GREEN logs in this directory;21 jobs tests and4 dispatch tests at latest focused run, plus wire/surface/plan tests. Full no-fail-fast suite and paid live mission still running. No model timer added or checker weakened. No push of this local slice yet.

Update21:45: production framed drain tests now exercise direct spill write failure and missing-status failure. Fault disabled observed RED, restored GREEN. Installer fake-bin inventories corrected after complete suite found2additional failures. Final workspace suite underway with expected root-only failures so far. Paid live job mission not yet cleared: critic used two broad filesystem searches then a non-streamed Messages provider request stalled in recvfrom (socket no byte progress since21:42). This is not the streamed HTTP-EOF bug and is not a live pass. Raw counters/process/trace evidence under artifacts/stall/jobs-critic. No model timer or reduced audit.

Final local suite:1192passed,7failed,1ignored,1200total. Only failures are3critic-shell and4repexec-sandbox tests requiring real root, unverified here. Native live job audit remains open, author served deepseek-flash. Proven mechanics slice only, not four tools exact-complete.

Push recovery: token lacks workflow permission, push rejected and remote stayed ad75d23. Renamed packaging helper to hs-plugin-shell-supervisor so existing CI hs-plugin-* glob already copies it. No workflow edit or account permission change needed. Installer/helper lookup/fixture tests updated; focused packaging tests green.
