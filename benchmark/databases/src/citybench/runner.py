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
from citybench.scenarios.registry import (
    ALL, SELECTIVITY_SCENARIOS, TIER3, ScenarioUnavailable, systems_for,
)

# Scenarios for which the `selectivity` column is left blank. Per the
# inherited contract (benchmark/formats/READ_BENCHMARK.md's CSV section):
# "selectivity = result_count / total_object_count, empty where N/A". These
# scenarios answer over the whole dataset, so a result_count/total ratio
# would not describe a selection at all; the write tier's `result_count` is
# rows TOUCHED by a mutation, which is not a selection either. Every other
# scenario DOES report it. Named here, once, so the rule is greppable
# rather than re-derived at each call site.
NO_SELECTIVITY_SCENARIOS = frozenset({"count", "geometry-scan"}) | set(TIER3)

#: Default relative spread below which a count disagreement is published as
#: an explained DEVIATION rather than a `mismatch` that fails the run.
#:
#: The bbox scenarios' counts differ across systems by 0.03-0.04 % for
#: reasons established in `notes/benchmark-fairness-review-2026-09-22.md`
#: §5 and now quoted in README Caveats 11 and 12: cjdb's importer drops
#: 2/16/60 BuildingPart footprints whose non-vertical faces share one Z,
#: and PostGIS's float4 `&&` admits 4 extra objects at the 25 % window on
#: BOTH PostgreSQL systems. These are properties of the compared systems,
#: not of the query, and every system is asked the same question with its
#: own idiomatic predicate. Treating them as a binary failure said nothing
#: a reader could act on; the decomposition does.
#:
#: Above the tolerance the row stays `mismatch` and the run still exits
#: non-zero — the check remains the harness's main defect detector.
DEFAULT_COUNT_TOLERANCE = 0.001


def cross_check(counts: dict[str, int],
                tolerance: float = DEFAULT_COUNT_TOLERANCE) -> tuple[str, str]:
    """``(note, status)`` for one scenario's counts across the systems.

    ``("", "ok")`` when every system agrees. Otherwise the note always
    describes the split — it is never dropped, whatever the status — and
    the status is ``"ok-deviation"`` when the relative spread
    ``(max - min) / max`` is within ``tolerance``, ``"mismatch"`` above it.
    """
    values = set(counts.values())
    if len(values) <= 1:
        return "", "ok"
    high, low = max(values), min(values)
    spread = 0.0 if high == 0 else (high - low) / abs(high)
    detail = " ".join(f"{tag}={count}" for tag, count in sorted(counts.items()))
    note = f"count-mismatch: {detail} spread={spread:.6f}"
    return note, ("ok-deviation" if spread <= tolerance else "mismatch")


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
               *, tolerance: float = DEFAULT_COUNT_TOLERANCE,
               run_note: str = "",
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
    by_tag = {system.tag: system for system in systems}

    for scenario in scenarios:
        wanted = [by_tag[tag] for tag in systems_for(scenario) if tag in by_tag]
        if not wanted:
            continue
        windows = (
            params.windows if scenario in SELECTIVITY_SCENARIOS else (None,)
        )

        for window in windows:
            # The window's own tag, suffixed `-approx` when the row-fraction
            # target was not reachable on this data, plus the fraction it
            # actually achieved — so a reader never has to assume "1 %"
            # meant 1 %.
            window_note = (
                f"{window.notes_tag()} achieved={window.achieved:.6f}"
                if window is not None else ""
            )
            measurements: dict[str, Measurement] = {}

            for system in wanted:
                try:
                    measurements[system.tag] = system.run(
                        scenario, params, repeat, window=window
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
            # The two `duckdb-cityparquet` package-ordering tags read the
            # same rows in a different order and the two write tags run the
            # same statements, so they are genuine participants in the
            # check rather than duplicates to exclude.
            deviation, status = cross_check(
                {tag: m.result_count for tag, m in answered.items()}, tolerance
            )

            for tag, m in measurements.items():
                note = " ".join(n for n in (run_note, window_note, deviation) if n)
                total, no_index = sizes.get(tag, (None, None))
                # Blank only for NO_SELECTIVITY_SCENARIOS; every other
                # scenario — including the non-windowed ones such as
                # attr-filter — reports result_count/total, per the
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
                        status=status,
                    )
                )

    return rows
