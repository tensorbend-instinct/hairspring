import os
import run_subset_par as R


def test_agent_env_puts_venv_bin_first():
    env = R.agent_env("/venvs/w2/pdm/bin/python", {"PATH": "/usr/bin:/bin"})
    assert env["PATH"].split(":")[0] == "/venvs/w2/pdm/bin"


def test_agent_env_keeps_shims_after_venv():
    env = R.agent_env("/venvs/w2/pdm/bin/python", {"PATH": "/usr/bin:/bin"})
    parts = env["PATH"].split(":")
    assert parts[1] == R.SHIMS
    assert "/usr/bin" in parts[2:]


def test_agent_env_does_not_mutate_base():
    base = {"PATH": "/usr/bin:/bin"}
    R.agent_env("/venvs/w2/pdm/bin/python", base)
    assert base["PATH"] == "/usr/bin:/bin"


def test_agent_env_carries_required_keys():
    env = R.agent_env("/venvs/w2/pdm/bin/python", {"PATH": "/usr/bin"})
    for k in ("HS_SWE_WORKSPACE", "HS_SWE_F2P", "HS_REALMODEL_CALL_TIMEOUT_SECS"):
        assert k in env
