import pytest
import run_subset_par as R


class CP:
    def __init__(self, rc=0, out="", err=""):
        self.returncode = rc
        self.stdout = out
        self.stderr = err


def mk_sh(collect_out="", collect_err="", exec_out="", import_path=""):
    def fake(cmd, cwd=None, timeout=None, env=None):
        if "--co -q" in cmd:
            return CP(0, collect_out, collect_err)
        if " -c " in cmd and "import" in cmd:
            return CP(0, import_path + "\n", "")
        return CP(0, exec_out, "")
    return fake


M = {"instance_id": "x__y-1", "repo": "deepset-ai/haystack"}
WS = "/tmp/ws-demo"
NODES = ["test/a.py::t1", "test/a.py::t2"]


def test_happy_path(monkeypatch):
    monkeypatch.setattr(R, "sh", mk_sh(
        collect_out="test/a.py::t1\ntest/a.py::t2\n\n2 tests collected\n",
        exec_out="2 failed in 0.5s\n",
        import_path=WS + "/haystack/__init__.py"))
    assert R.preflight_gate(M, WS, "/venv/bin/python", NODES) is None


def test_zero_collected_refuses(monkeypatch):
    monkeypatch.setattr(R, "sh", mk_sh(collect_out="", exec_out="2 failed in 0.5s\n",
                                       import_path=WS + "/haystack/__init__.py"))
    err = R.preflight_gate(M, WS, "/venv/bin/python", NODES)
    assert err and "preflight_gate_invalid" in err and "collect" in err


def test_collection_error_refuses(monkeypatch):
    monkeypatch.setattr(R, "sh", mk_sh(
        collect_out="ERROR test/a.py\n!!!! Interrupted: 1 error during collection !!!!\n",
        exec_out="2 failed in 0.5s\n", import_path=WS + "/haystack/__init__.py"))
    err = R.preflight_gate(M, WS, "/venv/bin/python", NODES)
    assert err and "preflight_gate_invalid" in err


def test_missing_node_refuses(monkeypatch):
    monkeypatch.setattr(R, "sh", mk_sh(collect_out="test/a.py::t1\n\n1 test collected\n",
                                       exec_out="1 failed in 0.5s\n",
                                       import_path=WS + "/haystack/__init__.py"))
    err = R.preflight_gate(M, WS, "/venv/bin/python", NODES)
    assert err and "preflight_gate_invalid" in err


def test_zero_executed_refuses(monkeypatch):
    monkeypatch.setattr(R, "sh", mk_sh(
        collect_out="test/a.py::t1\ntest/a.py::t2\n\n2 tests collected\n",
        exec_out="found no collectors\n\nno tests ran in 0.1s\n",
        import_path=WS + "/haystack/__init__.py"))
    err = R.preflight_gate(M, WS, "/venv/bin/python", NODES)
    assert err and "preflight_gate_invalid" in err and "execut" in err


def test_errors_at_execute_refuse(monkeypatch):
    monkeypatch.setattr(R, "sh", mk_sh(
        collect_out="test/a.py::t1\ntest/a.py::t2\n\n2 tests collected\n",
        exec_out="2 errors in 0.3s\n", import_path=WS + "/haystack/__init__.py"))
    err = R.preflight_gate(M, WS, "/venv/bin/python", NODES)
    assert err and "preflight_gate_invalid" in err


def test_stale_editable_refuses(monkeypatch):
    monkeypatch.setattr(R, "sh", mk_sh(
        collect_out="test/a.py::t1\ntest/a.py::t2\n\n2 tests collected\n",
        exec_out="2 failed in 0.5s\n",
        import_path="/mnt/other-arm/runs/x__y-1/ws/haystack/__init__.py"))
    err = R.preflight_gate(M, WS, "/venv/bin/python", NODES)
    assert err and "preflight_gate_invalid" in err and "editable" in err


def test_bare_file_node_ok(monkeypatch):
    monkeypatch.setattr(R, "sh", mk_sh(
        collect_out="tests/plugins/test_tf1.py::A::x\ntests/plugins/test_tf1.py::TestPluginCanHandleUrlTF1::test_all_matchers_match[stream]\n\n33 tests collected\n",
        exec_out="28 passed, 5 failed in 3s\n", import_path=WS + "/streamlink/__init__.py"))
    assert R.preflight_gate(M, WS, "/venv/bin/python",
                            ["tests/plugins/test_tf1.py",
                             "tests/plugins/test_tf1.py::TestPluginCanHandleUrlTF1::test_all_matchers_match[stream]"]) is None
