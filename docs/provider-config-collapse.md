# Provider-Config Collapse (design, 2026-09-04)

Status: design only. User ruling: providers are configuration per the spec's
TOML intent. DO NOT touch the model path until the A/B/C completes
(comparability of the three arms comes first).

## Goal
One generic OpenAI-compatible chat-completions plugin. Adding a provider is a
TOML entry, not new code. hs-plugin-glm and hs-plugin-deepseek collapse into
it; per-provider quirks become config fields.

## Config shape (draft)
[[providers]]
name = "glm"
base_url = "https://api.z.ai/api/coding/paas/v4/chat/completions"  # or env indirection
model = "glm-5.3"
key_env = "HS_GLM_API_KEY"            # or key_file_env
extra_body_json = '{"reasoning_effort":"max"}'   # effort knobs differ per provider
max_tokens = 98304                     # budget semantics differ per provider
price_in_micros = 1400                 # per Mtoken, for metered cost
price_cached_micros = 260
price_out_micros = 4400

## Notes
- realmodel.rs already IS this core: Provider struct + env indirection; the
  collapse lifts the two consts into TOML and teaches the kernel/loop to
  resolve providers from config instead of a hardcoded table.
- Measured quirks that must stay expressible: GLM key only valid on the
  coding-plan endpoint; GLM reasoning consumes the output budget (max_tokens
  fix, 2026-09-04); reasoning_effort low/high/max (GLM) vs DeepSeek's own knob.
- Migration: keep hs-plugin-glm/deepseek as thin shims over the generic plugin
  for one gate, then retire.
