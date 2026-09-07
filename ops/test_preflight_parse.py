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
