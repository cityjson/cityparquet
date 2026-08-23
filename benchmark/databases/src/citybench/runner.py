"""Drive the (system x scenario) matrix and guard its correctness.

The cross-check is the highest-value defect detector in a cross-system
benchmark. If two systems return different counts for the same scenario
and the same parameters, at least one is answering a different question,
and its timing is meaningless. Such rows are tagged, never silently
published.

A scenario can also be legitimately unanswerable for a given dataset (for
example, ``hierarchy`` on a dataset with no parent/child relationships at
all). That is a dataset property, not a system failure, and is recorded
distinctly — ``skipped: ...`` rather than ``error: ...`` — so the results
table does not conflate "nothing to ask" with "the system crashed".
"""

from __future__ import annotations

from dataclasses import replace

from citybench.config import Measurement, Params
from citybench.report import row_from_measurement
from citybench.scenarios.registry import ALL, SELECTIVITY_SCENARIOS, ScenarioUnavailable

SELECTIVITY_TARGETS = (0.01, 0.05, 0.25)

# The window tag published in `notes`, matching the existing harness.
_WINDOW_TAGS = {0.01: "bbox-1pct", 0.05: "bbox-5pct", 0.25: "bbox-25pct"}

# Scenarios for which the `selectivity` column is left blank. Per the
# inherited contract (benchmark/formats/READ_BENCHMARK.md's CSV section):
# "selectivity = result_count / total_object_count, empty where N/A (count,
# full-read)". Both of these scenarios answer over the whole dataset, so a
# result_count/total ratio would not describe a selection at all; every
# other scenario — attr-filter, attr-stats, id-lookup, project, bbox-query,
# and the tier-2 additions — DOES report it. Named here, once, so the rule
# is greppable rather than re-derived at each call site.
NO_SELECTIVITY_SCENARIOS = frozenset({"count", "full-read"})


def cross_check(counts: dict[str, int]) -> str:
    """Empty string when every system agrees; a note describing the split otherwise."""
    if len(set(counts.values())) <= 1:
        return ""
    detail = " ".join(f"{tag}={count}" for tag, count in sorted(counts.items()))
    return f"count-mismatch: {detail}"


def _selectivity(result_count: int | None, params: Params) -> float | None:
    """Selectivity is result_count / total CityObjects, per the spec.

    The window-area target is NOT this value — it goes in `notes` — because
    a 25% window does not select 25% of objects, and conflating the two
    would mislabel every bbox row.
    """
    if result_count is None or not params.total_city_objects:
        return None
    return result_count / params.total_city_objects


def _add_note(measurement: Measurement, note: str) -> Measurement:
    if not note:
        return measurement
    combined = f"{measurement.notes} {note}".strip()
    return replace(measurement, notes=combined)


def _failed(note: str) -> Measurement:
    return Measurement(
        result_count=None, times_s=[], server_times_s=[],
        peak_rss_bytes=None, notes=note,
    )


def run_matrix(systems, params: Params, dataset_name: str, repeat: int,
               scenarios: tuple[str, ...] = ALL,
               sizes: dict[str, tuple[int, int]] | None = None,
               ) -> list[dict[str, str]]:
    """Run every scenario on every system, cross-checking counts.

    A scenario with a window target expands into one row per target. Rows
    whose systems disagree on the count are tagged; a failing system yields
    a row recording the failure rather than disappearing from the table —
    a system that cannot answer is a result, not an absence.

    Two distinct kinds of "did not produce a count" are recorded separately:

    - ``ScenarioUnavailable`` (raised by a scenario's SQL builder when the
      dataset lacks what the scenario needs, e.g. no parent/child
      hierarchy) yields ``notes`` starting ``"skipped: "`` followed by the
      exception's message.
    - Any other exception yields ``notes`` starting ``"error: "`` followed
      by the exception's type name.

    Both kinds carry ``result_count=None`` and are excluded from the count
    cross-check — there is nothing to compare a missing count against.

    ``sizes`` maps a system tag to ``(size_bytes, size_bytes_no_index)``;
    those figures are stamped onto every row of that system so the CSV is
    self-contained for plotting size against query time.

    ``selectivity`` is populated as ``result_count / total_city_objects``
    for every scenario except those in ``NO_SELECTIVITY_SCENARIOS`` (
    ``count`` and ``full-read``), per the inherited CSV contract — see
    that constant's docstring for the exact wording.
    """
    sizes = sizes or {}
    rows: list[dict[str, str]] = []

    for scenario in scenarios:
        targets = SELECTIVITY_TARGETS if scenario in SELECTIVITY_SCENARIOS else (None,)

        for target in targets:
            window_note = _WINDOW_TAGS[target] if target is not None else ""
            measurements: dict[str, Measurement] = {}

            for system in systems:
                try:
                    measurements[system.tag] = system.run(
                        scenario, params, repeat, selectivity=target
                    )
                except ScenarioUnavailable as exc:
                    # A dataset property, not a system failure — kept
                    # distinct from `error:` so the paper can tell "nothing
                    # to ask" apart from "the system crashed".
                    measurements[system.tag] = _failed(f"skipped: {exc}")
                except Exception as exc:  # a system that cannot answer is a result
                    measurements[system.tag] = _failed(f"error: {type(exc).__name__}")

            answered = {
                tag: m for tag, m in measurements.items() if m.result_count is not None
            }
            mismatch = cross_check(
                {tag: m.result_count for tag, m in answered.items()}
            )

            for tag, m in measurements.items():
                note = " ".join(n for n in (window_note, mismatch) if n)
                total, no_index = sizes.get(tag, (None, None))
                # Blank only for `count`/`full-read` (NO_SELECTIVITY_SCENARIOS);
                # every other scenario — including the non-windowed ones such
                # as attr-filter — reports result_count/total, per the
                # inherited CSV contract this harness must concatenate with.
                selectivity = (
                    None if scenario in NO_SELECTIVITY_SCENARIOS
                    else _selectivity(m.result_count, params)
                )
                rows.append(
                    row_from_measurement(
                        dataset=dataset_name,
                        fmt=tag,
                        scenario=scenario,
                        measurement=_add_note(m, note),
                        selectivity=selectivity,
                        size_bytes=total,
                        size_bytes_no_index=no_index,
                    )
                )

    return rows
