import pytest

from citybench.stats import mad, median, standard_deviation


def test_median_odd_length():
    assert median([3.0, 1.0, 2.0]) == 2.0


def test_median_even_length_averages_middle_pair():
    assert median([1.0, 2.0, 3.0, 4.0]) == 2.5


def test_mad_is_median_of_absolute_deviations():
    # median is 3.0; deviations are [2,1,0,1,2]; median of those is 1.0
    assert mad([1.0, 2.0, 3.0, 4.0, 5.0]) == 1.0


def test_standard_deviation_is_population_not_sample():
    # [1, 2, 3] has population stdev sqrt(2/3) ~= 0.816497, sample stdev 1.0.
    assert standard_deviation([1.0, 2.0, 3.0]) == pytest.approx(0.816496580927726)


def test_mad_of_identical_values_is_zero():
    assert mad([2.5, 2.5, 2.5]) == 0.0


def test_empty_input_raises():
    with pytest.raises(ValueError):
        median([])
    with pytest.raises(ValueError):
        standard_deviation([])
    with pytest.raises(ValueError):
        mad([])


from citybench.stats import quantile, timing_summary  # noqa: E402


def test_timing_summary_even_count_interpolates_median_and_quartiles():
    s = timing_summary([4.0, 1.0, 3.0, 2.0])
    assert s["median"] == pytest.approx(2.5)
    assert s["q1"] == pytest.approx(1.75)
    assert s["q3"] == pytest.approx(3.25)
    assert (s["min"], s["max"]) == (1.0, 4.0)
    assert s["mean"] == pytest.approx(2.5)


def test_timing_summary_odd_count_hits_samples():
    s = timing_summary([5.0, 3.0, 1.0, 4.0, 2.0])
    assert (s["median"], s["q1"], s["q3"]) == (3.0, 2.0, 4.0)
    assert (s["min"], s["max"]) == (1.0, 5.0)


def test_timing_summary_single_sample_collapses_to_it():
    s = timing_summary([0.7])
    assert s == {"mean": 0.7, "std": 0.0, "median": 0.7, "min": 0.7,
                 "max": 0.7, "q1": 0.7, "q3": 0.7}


def test_timing_summary_std_is_population():
    assert timing_summary([1.0, 3.0])["std"] == pytest.approx(1.0)


def test_timing_summary_keys_follow_the_csv_block_order():
    assert list(timing_summary([1.0])) == ["mean", "std", "median", "min", "max", "q1", "q3"]


def test_quantile_matches_numpy_linear_default():
    assert quantile([1.0, 2.0, 3.0, 4.0], 0.25) == pytest.approx(1.75)
    with pytest.raises(ValueError):
        quantile([], 0.5)
