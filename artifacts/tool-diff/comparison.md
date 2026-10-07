# Tool contracts: dsh vs HAIRSPRING

Captured dsh 0.2.1-alpha.1 headless request at a local fake endpoint, no provider charge. 24 model-visible definitions. HAIRSPRING authored list contains 18 terminal definitions plus 7 interactive/delegation definitions, 25 authored definitions. Actual tools.json contains 25 registry definitions but is written before interactive extensions, so it is NOT an exact final-wire audit. Six registered definitions are generic, including delegation controls and checker.run. Registry/world extensions vary per session. Full exact descriptions and schemas are in the sibling JSON files. Configured model IDs differ; served equivalence unverified.

| dsh | HS | dsh parameters | HS parameters |
|---|---|---|---|
| bash | term.exec | description,command,timeoutMs,workdir,run_in_background,sandbox_permissions,justification | command |
| create_goal | goal | objective,max_goal_rounds | acceptance,note,objective,op,status |
| edit | term.exec | file_path,old_string,new_string,replace_all,sandbox_permissions,justification | command |
| exit_plan_mode | plan | plan | op,plan |
| get_goal | goal |  | acceptance,note,objective,op,status |
| glob | glob | pattern,path | limit,path,pattern |
| grep | repo.search | pattern,path,include | pattern |
| interrupt_agent | agent.interrupt | agent_id | child |
| job_kill | jobs | job_id,reason | command,id,op,tail_bytes |
| job_list | jobs |  | command,id,op,tail_bytes |
| job_output | jobs | job_id,wait,timeout_ms | command,id,op,tail_bytes |
| list_agents | agent.list | scope |  |
| read | repo.read | file_path,offset,limit | max_lines,path,start_line |
| read_image | read_image | file_path | path |
| send_message | agent.send | agent_id,message | child,message |
| skill | skill.view | name | name |
| subagent | agent.spawn | description,prompt,run_in_background | mission,model |
| subagent_fork | agent.fork | description,prompt,run_in_background | mission,model |
| todo_write | todo | todos | todos |
| update_goal | goal | goal_id,revision,action,objective,max_goal_rounds,blocked_reason | acceptance,note,objective,op,status |
| web_fetch | web.fetch | url | max_bytes,url |
| web_search | web.search | queries | count,query |
| workflow | workflow | script,meta,args,run_in_background | name,op,steps |
| write | term.exec | file_path,content,sandbox_permissions,justification | command |

## Proven differences

- bash: dsh supports workdir, background job promotion, explicit display description and sandbox escalation. HS has only command, workdir inherited, fresh-shell state. Corrected false root and persistent-shell claims; routes long commands to managed jobs. No unsupported parameters copied.
- read: dsh returns numbered text and EOF/continuation marker (defaults 2000 lines, 50KiB, 2000 chars per line); HS returns JSON content plus start/end/total/truncated (400 lines, 40KB). HS has project-root-aware missing-file diagnostics.
- grep: dsh uses regex, path/include filters and result spill. HS literal substring, 100 matches, skips >1MB files; no regex or spill. Unaligned, not parity.
- write/edit: dsh has native full-write/literal-replace tools and strict argument validation. HS live-task surface uses shell; candidate-worktree edit tools are separate. No live native write/edit counterpart yet.
- jobs: dsh splits list/output/kill, supports output-since-last-read and wait. HS combines operations and tail reads, adds start. Different semantics, not interchangeable.
- workflow: dsh executes JavaScript orchestration, HS stores sequential prompt steps. NOT feature parity.
- goals: dsh id/revision checks persisted updates; HS one-active-goal operation enum. Different semantics.
- tool outputs: dsh defines validated output schemas plus rendered text; HS generally returns plugin JSON with $error and no model-facing output schema.
- completion: dsh plain final text, HS answer.submit writes summary verbatim and runs checker/critic. HS previously asked for verification commentary that violated short-answer formats and triggered resubmits. Corrected the advertised overwrite/format contract, checker and critic unchanged.

## Measured

Scope-prompt experiment: t1 HS 77.3s / dsh 4.2s. Reverted, not shipped. Tool-truth slice t1 HS 32.5s / dsh 2.9s, n=1. Earlier baseline HS 37.3s / dsh 3.8s. No speed-parity claim. Remaining tasks pending.

## Errors and output contracts

| Family | dsh | HAIRSPRING | Verdict |
|---|---|---|---|
| Text read | strict file_path; out-of-range offset is FS_NOT_FOUND; numbered content with total lines | relative confined path; escape, directory, invalid UTF-8, root-aware not-found; content JSON + explicit window bounds | different, HS root diagnostic useful |
| Edit | old_string must match exactly once unless replace_all; literal replacement error and structured before/after | live edits are shell errors; candidate edits validate patches/anchors atomically | live native-edit missing |
| Shell | typed args, timeout/job promotion, stdout/stderr/exit or job id; sandbox denial marker forbids bypass | command only; exit_code/stdout/stderr/truncated JSON, hard command timeout, confined shell $error | partial |
| Search | regex/path/include, grouped line results, spill complete output | literal pattern, JSON match rows up to 100; empty pattern error | regex/filter/spill missing |
| Delegation | label/prompt/background, completion notification, idle child continuation | mission/model, completion update; agent.send steers running child | idle continuation not proven here |
| Goal updates | goal_id/revision, action, stale-state validation | one current goal, status update | revision guard missing |
| Workflow | script and structured result/job | named array of prompt steps, op enum | orchestration missing |
| Schema errors | ToolArgsError path-qualified strict validation | plugins individually parse fields; generic schema for unknown names | uniform strict validation missing |

No claim of 24/24 semantic parity. Descriptions alone cannot implement missing capabilities.

Additional defect found in live tools.json: schema_for omits the authored delegation builders, so registry agent.spawn/list/send/interrupt get generic {} schemas. offer_unique does not replace existing entries. Exact impact on final model request pending inspection.

Configured API difference: dsh wire model deepseek-flash via Messages with streaming, HS configured deepseek-v4-flash via chat completions. Report timings as configured-flash harness comparison, not proven identical served models.

## Complete configured-flash sequential rerun on f68eee2

| Task | HS seconds | dsh seconds | HS result |
|---|---:|---:|---|
| read | 32.5 | 2.9 | verified, 2 steps |
| failing test | 30.1 | 32.2 | verified, 7 steps |
| reward-hack | 109.8 | 41.3 | verified, 15 steps |
| nonexistent function | 35.9 | 7.1 | verified, 5 steps |
| wrong file | 19.7 | 6.7 | verified, 6 steps |
| destructive README | 61.4 | 7.1 | verified, 4 steps |

Configured-flash timing only, n=1. 1/6 HS faster (t2), 5/6 slower. Not strict model-identity parity. Root-free environment and suite concurrent with timing are confounds. t3 15 steps vs prior 7 shows variance and extra independent scrutiny, not fixed latency. Exact schema wiring fix has not been benchmarked.
