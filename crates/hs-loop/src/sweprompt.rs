//! SWE mission prompt as POLICY (spec gate 8: "the agent rewrites a prompt
//! and a tool"; bootstrap rule: promotion is gated until assay maturity).
//! The mission prompt template lives in the policy layer: a [prompts]
//! swe-mission TOML overlay replaces the builtin template. The model may
//! PROPOSE a new prompt in-mission; proposals are versioned, hash-chained,
//! and recorded with status "proposed" - they never mutate the running
//! mission. Promotion happens out-of-mission through the gated path
//! (gateway/human sign-off until the assay maturity gate passes).

use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The builtin template. Placeholders: {ws} {problem_statement}
/// {fail_to_pass} {repo_layout} {answer_path} {nudge}. Unknown placeholders
/// are left intact so policy authors can extend the arg set additively.
pub const SWE_MISSION_TEMPLATE: &str = "You are fixing a real bug in the repository checked out at {ws} (base commit, failing tests already added).\n\
MACHINE: you are on a real Linux box as root, not a toy sandbox. Network: ON (outbound and loopback; package installs fine). System roots are writable - apt-get/pip/cargo/npm all work. Host-only paths stay hidden (/home, /mnt). Detected tooling: {orientation}\n\
PROBLEM STATEMENT (from the issue tracker):\n{problem_statement}\n\n\
The checker will apply your patch and run: {fail_to_pass}\n\
It also runs a set of PASS_TO_PASS regression tests; do not break existing behavior.\n\n\
Repo files (partial listing):\n{repo_layout}\n\
TOOLS (one tool call per reply, exactly one JSON object, no prose):\n\
1. {{\"tool\":\"repo.search\",\"args\":{{\"pattern\":\"<literal substring>\"}}}} - find code by substring; returns path:line hits (max 100).\n\
2. {{\"tool\":\"repo.read\",\"args\":{{\"path\":\"<repo-relative path>\",\"start_line\":<1-indexed, optional>,\"max_lines\":<optional, default 400>}}}} - read a file window. The reply tells you total_lines and a truncated flag; if truncated, page forward with start_line=end_line+1. NEVER re-read the same window: recent results stay verbatim in your TRANSCRIPT, older work is distilled into the LEDGER block (always shown above), and exact duplicate reads are flagged with their earlier seq. Put durable facts (hypotheses, line numbers, failing tests) in notes.scratch.\n\
3. {{\"tool\":\"repo.exec\",\"args\":{{\"command\":\"<any command>\",\"diff\":\"<unified diff, optional>\",\"path\":\"<ANSWER_PATH, optional>\"}}}} - run lint/tests on a candidate patch inside a sandbox (applied to a scratch copy; the repo stays clean; full machine floor: network on, system roots writable, you are root). Pass diff INLINE to test a candidate BEFORE writing any answer; pass path (or nothing) to test the current answer file. If the patch does not apply you get the git error back free - fix the framing before spending a checker cycle. Run the FAIL_TO_PASS command before every answer.write.\n\
6. {{\"tool\":\"edit.apply\",\"args\":{{\"diff\":\"<unified diff>\"}}}} - apply one incremental edit to your persistent candidate workspace (the live repo is never touched). Returns the CUMULATIVE diff of everything you have applied so far: use edit.apply as you work, test with repo.exec, and submit the cumulative result. ops: {{\"op\":\"diff\"}} re-reads the cumulative diff, {{\"op\":\"reset\"}} discards the candidate.\n\
7. {{\"tool\":\"notes.scratch\",\"args\":{{\"op\":\"write|append|read\",\"content\":\"<text>\"}}}} - persistent notes that survive context truncation. Record hypotheses, failing test names, and line numbers you will need later; read them back instead of re-discovering.\n\'

4. {{\"tool\":\"policy.propose_prompt\",\"args\":{{\"name\":\"swe-mission\",\"text\":\"<your improved prompt template>\"}}}} - propose a better operating prompt for FUTURE missions. Recorded, versioned, and reviewed through the gated promotion path; it never changes this mission.\n\
5. {{\"tool\":\"answer.write\",\"args\":{{\"path\":\"<ANSWER_PATH>\",\"content\":\"```diff\\n<one unified diff, paths a/... b/... relative to repo root>\\n```\"}}}} - submit your patch. Ground every hunk in code you actually read: correct file, correct current line numbers, exact context lines. Prefer a repo.exec pre-flight first. The checker runs automatically after each answer.write and its verdict comes back as FEEDBACK.\n\
WORKFLOW: search and read to locate the real code FIRST, then write a patch that applies cleanly. \
The exact ANSWER_PATH value is given to you on the ANSWER_PATH line each attempt. \
Do not include prose outside the JSON. If you get FEEDBACK, repair and continue.{nudge}";

pub struct PromptArgs {
    pub ws: String,
    pub problem_statement: String,
    pub fail_to_pass: Vec<String>,
    pub repo_layout: String,
    pub nudge: String,
    pub answer_path: String,
    /// Detected tooling for the MACHINE orientation line (fix 2); callers
    /// fill it with probe_orientation().
    pub orientation: String,
}

/// Probe the machine floor for the orientation brief (fix 2): the repo.exec
/// sandbox binds the real system roots, so host detection IS sandbox
/// detection.
pub fn probe_orientation() -> String {
    let tools = ["python3", "pip3", "cargo", "npm", "node", "apt-get", "git", "curl"];
    let mut have: Vec<&str> = vec![];
    for t in tools {
        let ok = std::process::Command::new("sh")
            .args(["-c", &format!("command -v {t} >/dev/null 2>&1")])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if ok {
            have.push(t);
        }
    }
    if have.is_empty() {
        "(none detected)".to_string()
    } else {
        have.join(", ")
    }
}

/// Policy overlay: [prompts] name = template. Loaded from TOML; malformed
/// config is an error, never a silent fallback to builtin.
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct PolicyOverlay {
    #[serde(default)]
    pub prompts: BTreeMap<String, String>,
}

pub fn load_policy_overlay(path: &Path) -> Result<PolicyOverlay, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("policy overlay {}: {e}", path.display()))?;
    toml::from_str(&text).map_err(|e| format!("policy overlay {}: {e}", path.display()))
}

fn substitute(template: &str, args: &PromptArgs) -> String {
    let f2p = args.fail_to_pass.join(" ; ");
    let pairs = [
        ("{ws}", args.ws.as_str()),
        ("{problem_statement}", args.problem_statement.trim()),
        ("{fail_to_pass}", f2p.as_str()),
        ("{repo_layout}", args.repo_layout.as_str()),
        ("{answer_path}", args.answer_path.as_str()),
        ("{nudge}", args.nudge.as_str()),
        ("{orientation}", args.orientation.as_str()),
    ];
    let mut out = template.to_string();
    for (k, v) in pairs {
        out = out.replace(k, v);
    }
    out
}

pub fn build_mission_prompt(policy: Option<&PolicyOverlay>, args: &PromptArgs) -> String {
    let template = policy
        .and_then(|p| p.prompts.get("swe-mission"))
        .map(String::as_str)
        .unwrap_or(SWE_MISSION_TEMPLATE);
    substitute(template, args)
}

/// One recorded proposal. version is 1-based over the proposal log; hash
/// chains to parent_hash so the lineage is tamper-evident on the event
/// stream (spec: lineage records mutation).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ProposalRecord {
    pub version: u64,
    pub parent_hash: String,
    pub hash: String,
    pub name: String,
    pub text: String,
    pub status: String, // always "proposed"; promotion is out-of-mission
}

fn proposals_path(dir: &Path) -> PathBuf {
    dir.join("policy_proposals.jsonl")
}

pub fn content_hash(s: &str) -> String {
    // FNV-1a 64: deterministic across processes, enough for lineage chaining
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub fn propose_prompt(dir: &Path, name: &str, text: &str) -> Result<ProposalRecord, String> {
    let path = proposals_path(dir);
    let mut version = 0u64;
    let mut parent_hash = "genesis".to_string();
    if let Ok(existing) = std::fs::read_to_string(&path) {
        for line in existing.lines().filter(|l| !l.trim().is_empty()) {
            let r: ProposalRecord = serde_json::from_str(line)
                .map_err(|e| format!("corrupt proposal log {}: {e}", path.display()))?;
            version = r.version;
            parent_hash = r.hash;
        }
    }
    let version = version + 1;
    let hash = content_hash(&format!("{version}|{parent_hash}|{name}|{text}"));
    let rec = ProposalRecord {
        version,
        parent_hash,
        hash,
        name: name.into(),
        text: text.into(),
        status: "proposed".into(),
    };
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("proposal log {}: {e}", path.display()))?;
    writeln!(f, "{}", json!(rec)).map_err(|e| format!("proposal log write: {e}"))?;
    Ok(rec)
}
