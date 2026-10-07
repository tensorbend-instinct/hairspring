from calc import average, clamp
def test_average():
    assert average([2, 4, 6]) == 4
def test_clamp():
    assert clamp(15, 0, 10) == 10
