# Gate-8 single-task box playbook (freeze re-roll recovery)

## State that lives OFF the box (survives a wipe)
- Repo: github.com/tensorbend-instinct/hairspring main (all code + this ops dir)
- Drive mirror file id: 1ch6ZYOyq4BCOlspiXgqJV7y-CC50w8CO (folder 02-results,
  anyone-with-link reader; updater = ops/.swe-mirror.sh)
- API keys: vault entries "GLM API key" + "DeepSeek API key" (password subfield
  only). Bridge to box: RSA-OAEP page (crypto.subtle needs secure context -
  use the BigInt RSA + manual PKCS1 v1.5 variant), vault fill, execute-js,
  openssl pkeyutl -decrypt locally, chmod 600. Never echo.
- Dataset: https://huggingface.co/api/datasets/SWE-bench-Live/SWE-bench-Live/parquet/default/lite/0.parquet

## Rebuild steps (fresh box)
1. git clone the repo; cargo build --workspace (rustup stable)
2. pip install pyarrow pytest
3. Bridge keys -> /home/sandbox/.keys/{glm.key,deepseek.key}
4. curl the dataset parquet to /downloads/swelive_lite.parquet
5. Export instance yt-dlp__yt-dlp-12684 to /home/sandbox/instance_12684.json
   + test_patch to instance_12684_testpatch.diff (pyarrow script in history)
6. Prep workspace: curl -L github.com/yt-dlp/yt-dlp/archive/2ee3a0aff9be2be3bea60640d3d8a0febaf0acb6.tar.gz
   -> /home/sandbox/swbench/single/ws; git init; commit; git apply test_patch; commit
   Verify: pytest test/test_jsinterp.py::TestJSInterpreter::test_extract_function_with_global_stack FAILS at base
7. GLM endpoint: HS_GLM_BASE_URL=https://api.z.ai/api/coding/paas/v4/chat/completions
   (coding-plan key; paas endpoint rejects it)
8. Launch: bash ops/.swe-launch.sh (sets HS_* env, seeds last answer)
   Supervisor: setsid bash ops/.swe-supervisor.sh (v6 freeze sentinel)
   Mirror: setsid bash ops/.swe-mirror.sh (Drive updater, ~60s)
9. Wake schedule on the platform side keeps monitoring (20-min cadence).

## Run parameters
- model glm (glm-5.3), feedback ON, budget 1_000_000 micros ($1), max-steps 25
- HS_REALMODEL_CALL_TIMEOUT_SECS=900
- F2P: python3 -m pytest test/test_jsinterp.py::TestJSInterpreter::test_extract_function_with_global_stack -x -q
