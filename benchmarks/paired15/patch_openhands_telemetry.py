#!/usr/bin/env python3
"""Apply the pinned LiteLLM/OpenHands cache-detail compatibility fix.

LiteLLM 1.79.x deletes unset cache_creation_tokens from its wrapper instance
but retains it in model_fields_set. OpenHands 1.49.6 dereferences that missing
attribute after checking model_fields_set. This patch does not change the
model response or proxy accounting, only optional SDK telemetry.
"""
import pathlib

root=pathlib.Path(__file__).resolve().parent
import os
venv=pathlib.Path(os.environ['HS_AB_OH_VENV']) if os.environ.get('HS_AB_OH_VENV') else root/'openhands-env12'
path=venv/'lib/python3.12/site-packages/openhands/sdk/llm/utils/telemetry.py'
s=path.read_text()
old='int(prompt_details.cache_creation_tokens or 0)'
new='int(getattr(prompt_details, "cache_creation_tokens", None) or 0)'
if old in s:
 assert s.count(old)==1
 path.write_text(s.replace(old,new))
elif new not in s:raise SystemExit('unexpected SDK telemetry source; refusing patch')
print('OpenHands telemetry compatibility patch installed')
