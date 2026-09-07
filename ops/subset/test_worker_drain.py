"""RED (2026-09-07, tail starvation): a worker thread that finds every rowless
mission momentarily claimed by a sibling RETURNED PERMANENTLY, draining the
pool to 1 live worker and stranding cfn-lint-3862 + conan-17302 with 15 idle
workers. A worker must wait+rescan while any manifest mission lacks
result.json; it may exit only when nothing remains."""
import os
import threading

import run_subset_par as R


def test_worker_waits_for_sibling_instead_of_exiting(tmp_path, monkeypatch):
    s50 = tmp_path / "s50"
    (s50 / "runs" / "a__a-1").mkdir(parents=True)
    claims = tmp_path / "claims"
    claims.mkdir()
    monkeypatch.setattr(R, "S50", str(s50))
    monkeypatch.setattr(R, "CLAIMS", str(claims))
    monkeypatch.setattr(R, "MANIFEST", [{"instance_id": "a__a-1", "repo": "x/y", "test_patch": ""}], raising=False)
    # a sibling holds the only mission's claim; no result yet
    (claims / "a__a-1").mkdir()

    done = []
    t = threading.Thread(target=lambda: (R.worker(9), done.append(True)), daemon=True)
    t.start()
    t.join(1.0)
    assert not done, "worker exited while a mission was still unclaimed-but-running (tail starvation)"

    # sibling finishes
    (s50 / "runs" / "a__a-1" / "result.json").write_text("{}")
    t.join(15)
    assert done, "worker never noticed the queue drained"
