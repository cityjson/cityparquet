import pytest

from citybench.stats import mad, median


def test_median_odd_length():
    assert median([3.0, 1.0, 2.0]) == 2.0


def test_median_even_length_averages_middle_pair():
    assert median([1.0, 2.0, 3.0, 4.0]) == 2.5


def test_mad_is_median_of_absolute_deviations():
    # median is 3.0; deviations are [2,1,0,1,2]; median of those is 1.0
    assert mad([1.0, 2.0, 3.0, 4.0, 5.0]) == 1.0


def test_mad_of_identical_values_is_zero():
    assert mad([2.5, 2.5, 2.5]) == 0.0


def test_empty_input_raises():
    with pytest.raises(ValueError):
        median([])
    with pytest.raises(ValueError):
        mad([])
