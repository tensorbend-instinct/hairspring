# Providers (models)

DeepSeek and GLM are built in. Any other OpenAI-compatible endpoint -
OpenRouter, OpenAI direct, a local server - is configuration, not code.
Three pieces, all under `~/.config/hairspring/`:

1. `providers.toml` declares the endpoint (prices feed the conservative
   cost ledger; keys never appear here):

```toml
[[providers]]
name = "openrouter"
base_url = "https://openrouter.ai/api/v1/chat/completions"
model = "openai/gpt-6-astra-pro"
key_env = "HS_OPENROUTER_API_KEY"
price_in_micros = 10.0
price_cached_micros = 1.0
price_out_micros = 50.0
```

2. A `[[models]]` block in `hairspring.toml` makes it pickable in the TUI
   (`/models`; the pick persists across restarts). The command is the
   generic provider plugin with the TOML name as its argument:

```toml
[[models]]
name = "openrouter"
command = ["@PREFIX@/bin/hs-plugin-provmodel", "openrouter"]
subjects = ["*"]
```

3. The key: `hairspring setup` lists every provider in `providers.toml`
   alongside the builtins, validates the key with one zero-cost
   round-trip, and saves it owner-only to `keys/<name>.key`.
   Non-interactive: `hairspring setup --provider openrouter --key-stdin`.
   Or export the env var the entry names. Resolution order: env var,
   `$HS_<NAME>_API_KEY_FILE`, `keys/<name>.key`.

OpenAI direct is the same shape:

```toml
[[providers]]
name = "openai"
base_url = "https://api.openai.com/v1/chat/completions"
model = "gpt-5"
key_env = "HS_OPENAI_API_KEY"
```

Per-provider env overrides win over the TOML values: `HS_<NAME>_MODEL`,
`HS_<NAME>_BASE_URL`, `HS_<NAME>_EXTRA_BODY_JSON` (e.g. reasoning
effort), `HS_<NAME>_PRICE_*_MICROS`. `HS_PROVIDERS_TOML` points at a
different providers file. The critic uses the mission model by default;
`HS_CRITIC_MODEL=<name>` pins it to any provider, builtin or TOML.
