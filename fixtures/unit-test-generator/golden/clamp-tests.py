import pytest
from your_module import clamp


def test_clamp_value_within_range():
    assert clamp(5.0, 0.0, 10.0) == 5.0


def test_clamp_value_at_min_boundary():
    assert clamp(0.0, 0.0, 10.0) == 0.0


def test_clamp_value_at_max_boundary():
    assert clamp(10.0, 0.0, 10.0) == 10.0


def test_clamp_value_below_min():
    assert clamp(-5.0, 0.0, 10.0) == 0.0


def test_clamp_value_above_max():
    assert clamp(15.0, 0.0, 10.0) == 10.0


def test_clamp_raises_when_min_greater_than_max():
    with pytest.raises(ValueError):
        clamp(5.0, 10.0, 0.0)


def test_clamp_equal_min_max():
    assert clamp(5.0, 3.0, 3.0) == 3.0
