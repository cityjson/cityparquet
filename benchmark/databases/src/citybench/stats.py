"""Robust summary statistics for benchmark timings.

The median and median absolute deviation are used rather than mean and
standard deviation because a benchmark sample set routinely contains
outliers from OS scheduling and background load, and the median is not
dragged by them.
"""

import statistics


def median(values: list[float]) -> float:
    """Median of ``values``. Raises ValueError if empty."""
    if not values:
        raise ValueError("median requires at least one value")
    return statistics.median(values)


def mad(values: list[float]) -> float:
    """Median absolute deviation from the median. Raises ValueError if empty."""
    if not values:
        raise ValueError("mad requires at least one value")
    centre = statistics.median(values)
    return statistics.median([abs(v - centre) for v in values])
