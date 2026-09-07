"""RED (2026-09-06, cfn-lint-3855 preflight over-refusal): the execute-pass
error counter scraped the WHOLE pytest output, so assertion message text like
"AssertionError: Expected 1 errors for Not equal string and boolean" (from
test_equals_is_useful.py) counted as a real pytest error and the preflight
refused a valid mission. Counts must come from pytest's final summary line
only; FAILED at base is expected (that IS the mission); genuine errors still
refuse."""
import os, sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "subset"))
import run_subset_par as rsp  # noqa: E402

OUT_3855 = """
test/unit/rules/conditions/test_equals_is_useful.py::test_names[Not equal string and boolean-instance4-1] FAILED
E       AssertionError: Expected 1 errors for Not equal string and boolean and [True, "true"]
E       assert 0 == 1
=========================== short test summary info ============================
FAILED test/unit/rules/conditions/test_equals_is_useful.py::test_names[Not equal string and boolean-instance4-1]
FAILED test/unit/rules/conditions/test_equals_is_useful.py::test_names[Not equal string and integer-instance3-1]
2 failed, 3 passed in 0.07s
"""

OUT_REAL_ERROR = """
=========================== short test summary info ============================
ERROR test/unit/rules/conditions/test_x.py::test_y - ModuleNotFoundError: no module named boom
1 error in 0.12s
"""


def test_assertion_message_text_is_not_a_pytest_error():
    counts = rsp.parse_exec_counts(OUT_3855)
    assert counts.get("error", 0) == 0, f"phantom error from assertion text: {counts}"
    assert counts.get("failed", 0) == 2
    assert counts.get("passed", 0) == 3


def test_genuine_execute_error_still_refuses():
    counts = rsp.parse_exec_counts(OUT_REAL_ERROR)
    assert counts.get("error", 0) == 1, f"real error missed: {counts}"

"""RED (2026-09-07, pdm-3374 preflight over-refusal): when the test patch's
own fixtures require the gold feature (pdm-3374's tests/cli/conftest.py
fixture calls get_auth_info.cache_clear(), a method the gold patch adds),
F2P tests ERROR at base BY DESIGN and the mission is completable. The gate
must refuse execute errors from BASE files (environment breakage) but accept
errors whose every named file is touched by test_patch.diff."""

OUT_3374 = """
tests/cli/conftest.py:123: AttributeError
----------------------------- Captured stdout setup -----------------------------
Changes are written to pyproject.toml.
=========================== short test summary info ============================
ERROR tests/cli/test_config.py::test_config_password_save_into_keyring - AttributeError: 'function' object has no attribute 'cache_clear'
ERROR tests/cli/test_publish.py::test_repository_get_credentials_from_keyring - AttributeError: 'function' object has no attribute 'cache_clear'
3 errors in 0.36s
"""

TP_3374 = """diff --git a/tests/cli/conftest.py b/tests/cli/conftest.py
index 1111111..2222222 100644
--- a/tests/cli/conftest.py
+++ b/tests/cli/conftest.py
diff --git a/tests/cli/test_config.py b/tests/cli/test_config.py
index 3333333..4444444 100644
--- a/tests/cli/test_config.py
+++ b/tests/cli/test_config.py
"""

OUT_BASE_FILE_ERROR = """
tests/cli/conftest.py:123: AttributeError
src/pdm/models/auth.py:41: AttributeError
=========================== short test summary info ============================
ERROR tests/cli/test_config.py::test_config_password_save_into_keyring - AttributeError: 'function' object has no attribute 'cache_clear'
2 errors in 0.28s
"""


def test_testpatch_intrinsic_errors_are_accepted():
    assert rsp.errors_are_testpatch_intrinsic(OUT_3374, TP_3374) is True


def test_base_file_errors_still_refuse():
    assert rsp.errors_are_testpatch_intrinsic(OUT_BASE_FILE_ERROR, TP_3374) is False


def test_no_traceback_frames_is_not_intrinsic():
    assert rsp.errors_are_testpatch_intrinsic("1 failed in 0.10s", TP_3374) is False
    assert rsp.errors_are_testpatch_intrinsic(
        "ERROR tests/cli/test_config.py::test_x - AttributeError: boom\n1 error in 0.1s",
        TP_3374) is False
