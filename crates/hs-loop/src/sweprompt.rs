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
pub const SWE_MISSION_TEMPLATE: &str = "You are fixing a real bug in the repository checked out at {ws} (base commit, failing tests already added). repo.exec sees the repo at /ws; every tool takes repo-relative paths.\n\
MACHINE: you are on a real Linux box as root, not a toy sandbox. {network_line} System roots are writable - apt-get/pip/cargo/npm all work. Host-only paths stay hidden (/home, /mnt). Detected tooling: {orientation}\n\
PROBLEM STATEMENT (from the issue tracker):\n{problem_statement}\n\n\
The checker applies your patch, then runs the FAIL_TO_PASS tests. Run them yourself inside repo.exec with EXACTLY this command (the interpreter is on PATH): {fail_to_pass}\n\
It also runs a set of PASS_TO_PASS regression tests; do not break existing behavior.\n\n\
Repo files (partial listing):\n{repo_layout}\n\
TOOLS: your tools arrive through the native tool-calling API - call exactly one per reply, no prose.\n\
WORK POLICY:\n\
- Claim that something is done, fixed, tested, or addressed only when tool output supports the claim. Otherwise state what you did not verify and why.\n\
- If something is blocked, say so plainly rather than quietly dropping it.\n\
- Do the work in the current step instead of ending with an offer to do it later.\n\
- {edit_policy}\n\
WORKFLOW: search and read to locate the real code FIRST, then build the fix with {edit_tool} and verify it with repo.exec before answer.submit. \
The exact ANSWER_PATH value is given to you on the ANSWER_PATH line each attempt. \
Do not include prose outside the JSON. If you get FEEDBACK, repair what it reports before resubmitting - resubmitting an answer the verifier already refuted earns another refutation, not acceptance.{nudge}";

pub const SWE_MISSION_BLIND_TEMPLATE: &str = "You are fixing a real bug in the repository checked out at {ws} (base commit). repo.exec sees the repo at /ws; every tool takes repo-relative paths.\n\
MACHINE: you are on a real Linux box as root, not a toy sandbox. {network_line} System roots are writable - apt-get/pip/cargo/npm all work. Host-only paths stay hidden (/home, /mnt). Detected tooling: {orientation}\n\
PROBLEM STATEMENT (from the issue tracker):\n{problem_statement}\n\n\
There is NO provided test suite: your own checks are the only gate. Write tests that would catch this bug, then declare the commands that run them - one per line - in .hs/checks at the repo root (harness machinery: the file never joins your submitted patch). The checker runs exactly those commands against your candidate and is green only when every one passes. Run them yourself with repo.exec before submitting; an audit of your recorded work follows every submission.\n\n\
Repo files (partial listing):\n{repo_layout}\n\
TOOLS: your tools arrive through the native tool-calling API - call exactly one per reply, no prose.\n\
WORK POLICY:\n\
- Claim that something is done, fixed, tested, or addressed only when tool output supports the claim. Otherwise state what you did not verify and why.\n\
- If something is blocked, say so plainly rather than quietly dropping it.\n\
- Do the work in the current step instead of ending with an offer to do it later.\n\
- {edit_policy}\n\
WORKFLOW: search and read to locate the real code FIRST, then build the fix with {edit_tool} and verify it with repo.exec before answer.submit. \
The exact ANSWER_PATH value is given to you on the ANSWER_PATH line each attempt. \
Do not include prose outside the JSON. If you get FEEDBACK, repair what it reports before resubmitting - resubmitting an answer the verifier already refuted earns another refutation, not acceptance.{nudge}";

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
    /// Registered MCP tools, rendered for the TOOLS section (graft-experiment
    /// finding: kernel-registered but prompt-absent tools are invisible to
    /// the model). Empty when no MCP servers are configured.
    pub mcp_tools: String,
}

/// Probe the machine floor for the orientation brief (fix 2): the repo.exec
/// sandbox binds the real system roots, so host detection IS sandbox
/// detection.
pub fn probe_orientation() -> String {
    let tools = [
        "python3", "pip3", "cargo", "npm", "node", "apt-get", "git", "curl",
    ];
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
    let editpol = edit_policy();
    let pairs = [
        ("{ws}", args.ws.as_str()),
        ("{problem_statement}", args.problem_statement.trim()),
        ("{fail_to_pass}", f2p.as_str()),
        ("{edit_policy}", editpol.as_str()),
        (
            "{network_line}",
            if crate::repexec::egress_off() {
                "Network: OFF (egress blocked - no package installs, no fetches; the repo, the venv, and system tooling are all you have)."
            } else {
                "Network: ON (outbound and loopback; package installs fine)."
            },
        ),
        (
            "{edit_tool}",
            if std::env::var("HS_SWE_EDIT_PATH").as_deref() == Ok("anchor") {
                "edit.anchor"
            } else {
                "edit.patch"
            },
        ),
        ("{repo_layout}", args.repo_layout.as_str()),
        ("{answer_path}", args.answer_path.as_str()),
        ("{nudge}", args.nudge.as_str()),
        ("{orientation}", args.orientation.as_str()),
        ("{mcp_tools}", args.mcp_tools.as_str()),
    ];
    let mut out = template.to_string();
    for (k, v) in pairs {
        out = out.replace(k, v);
    }
    out
}

/// Blind mode (Eric 2026-09-07: "no fail to pass - that's cheating"): the
/// mission prompt teaches self-verification via .hs/checks; ground-truth
/// FAIL_TO_PASS text never appears in any form. fail_to_pass in `args` is
/// ignored by construction - the blind template has no placeholder for it.
pub fn build_blind_mission_prompt(policy: Option<&PolicyOverlay>, args: &PromptArgs) -> String {
    let template = policy
        .and_then(|p| p.prompts.get("swe-mission-blind"))
        .map(String::as_str)
        .unwrap_or(SWE_MISSION_BLIND_TEMPLATE);
    substitute(template, args)
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

/// The edit-path policy line (bake-off, 2026-09-06): selected per mission
/// via HS_SWE_EDIT_PATH (default applypatch).
fn edit_policy() -> String {
    if std::env::var("HS_SWE_EDIT_PATH").as_deref() == Ok("anchor") {
        "Make ALL edits with edit.anchor (anchor ops on the LINE:HASH prefixes repo.read shows: replace/insert_after/write; quote anchors exactly - stale or wrong anchors are named errors and nothing is half-applied) - never git apply, never hand-written .diff/.patch files; repo.exec is build/test only. Rejected bypass attempts are counted per class and escalate - never retry a rejected class.".to_string()
    } else {
        "Make ALL edits with edit.patch (Codex apply_patch grammar: *** Begin Patch, *** Update File/Add File/Delete File, *** End Patch; context lines copied verbatim from repo.read) - never git apply, never hand-written .diff/.patch files; repo.exec is build/test only. Rejected bypass attempts are counted per class and escalate - never retry a rejected class.".to_string()
    }
}
