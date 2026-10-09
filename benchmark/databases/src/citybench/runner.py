"""Drive the (system x scenario) matrix and guard its correctness.

The cross-check is the highest-value defect detector in a cross-system
benchmark. If two systems return different counts for the same scenario
and the same parameters, at least one is answering a different question,
and its timing is meaningless. Such rows are tagged, never silently
published.

A scenario can also be legitimately unanswerable for a given dataset (for
example, ``attr-stats`` on a dataset with no numeric attribute at all, or
``append-object`` on a plain CityJSON source no single feature can be cut
out of). That is a dataset property, not a system failure, and is recorded
distinctly — ``skipped: ...`` rather than ``error: ...`` — so the results
table does not conflate "nothing to ask" with "the system crashed".
"""

from __future__ import annotations

import sys
from dataclasses import replace

from citybench import identity
from citybench.config import BboxWindow, IdProbe, Measurement, Params
from citybench.report import row_from_measurement
from citybench.scenarios.registry import (
    ALL, ID_PROBE_SCENARIOS, SELECTIVITY_SCENARIOS, TIER3,
    ScenarioUnavailable, systems_for,
)

# Scenarios for which the `selectivity` column is left blank. Per the
# inherited contract (benchmark/formats/READ_BENCHMARK.md's CSV section):
# "selectivity = result_count / total_object_count, empty where N/A".
# `geometry-scan` answers over the whole dataset, so a result_count/total ratio
# would not describe a selection at all; the write tier's `result_count` is
# rows TOUCHED by a mutation, which is not a selection either. Every other
# scenario DOES report it. Named here, once, so the rule is greppable
# rather than re-derived at each call site.
NO_SELECTIVITY_SCENARIOS = frozenset({"geometry-scan"}) | set(TIER3)

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
        peak_working_mem_bytes=None, notes=note,
    )


def variants(scenario: str, params: Params
             ) -> list[tuple[BboxWindow | None, IdProbe | None]]:
    """The `(window, probe)` pairs ``scenario`` expands into.

    Most scenarios are measured once and yield `[(None, None)]`.
    `bbox-query` is measured at each of the three row-fraction windows, and
    `id-lookup` at its two id probes — the middle-position hit and a
    verified-absent id. Each pair becomes its own CSV row on every system,
    and therefore its own cross-system count check: the hit's count is
    compared against the hit's, never against a blend with the miss.
    """
    if scenario in SELECTIVITY_SCENARIOS:
        return [(window, None) for window in params.windows]
    if scenario in ID_PROBE_SCENARIOS:
        return [(None, probe) for probe in params.id_probes]
    return [(None, None)]


def _variant_note(window: BboxWindow | None, probe: IdProbe | None) -> str:
    if window is not None:
        return f"{window.notes_tag()} achieved={window.achieved:.6f}"
    if probe is not None:
        return probe.notes_tag()
    return ""


def explained_footprint_deviation(scenario: str,
                                  summaries: dict[str, identity.Identity],
                                  systems: list) -> str | None:
    """The one deviation the identifier check accepts, verified per row.

    cjdb's `bbox-query` tests the object's footprint (`ground_geometry`),
    the only spatial column it has, while the other systems test the box
    over the object's subtree; cjdb cannot test that box through an index
    (README Caveats 10-11). The row is accepted only when every other
    system returns the identical set, cjdb's set is a subset of it, and the
    cjdb system decomposes each missing id by its footprint (NULL, or not
    meeting the window); the note carries that decomposition.
    """
    if scenario != "bbox-query":
        return None
    for system in systems:
        if (system.tag not in summaries
                or not hasattr(system, "footprint_decomposition")):
            continue
        others = [s.ids for tag, s in summaries.items() if tag != system.tag]
        own = summaries[system.tag].ids
        if not others or any(o != others[0] for o in others) or not own < others[0]:
            return None
        missing = sorted(others[0] - own)
        parts = system.footprint_decomposition(missing)
        if sum(parts.values()) != len(missing):
            return None
        detail = " ".join(f"{k}={v}" for k, v in parts.items())
        return (f"explained-deviation: {system.tag} tests its footprint, lacks "
                f"{len(missing)} of {len(others[0])} ({detail}; README Caveats 10-11)")
    return None


def _verify_identities(scenario: str, params: Params, window, probe,
                       systems: list, answered: dict[str, Measurement],
                       status: str, tolerance: float) -> tuple[str | None, str | None]:
    """The identifier-set cross-check for one row (`citybench.identity`).

    Runs AFTER the timed samples: each answering system that implements
    `verify_rows` executes the scenario's SQL once more, untimed, and the
    identifier sets and non-null geometry counts are compared. The count
    tolerance applies only where the count cross-check already accepted a
    deviation.
    """
    if scenario not in identity.IDENTITY_SCENARIOS:
        return None, None
    summaries: dict[str, identity.Identity] = {}
    for system in systems:
        if system.tag not in answered or not hasattr(system, "verify_rows"):
            continue
        columns, rows = system.verify_rows(scenario, params, window=window, probe=probe)
        summaries[system.tag] = identity.summarise(scenario, columns, rows)
    explained = explained_footprint_deviation(scenario, summaries, systems)
    if explained:
        print(f"EXPLAINED {scenario}: {explained}", file=sys.stderr, flush=True)
        return None, explained
    note = identity.compare(
        summaries, tolerance if status == "ok-deviation" else 0.0
    )
    if note:
        print(f"ID MISMATCH {scenario}: {note}", file=sys.stderr, flush=True)
    return note, None


def run_matrix(systems, params: Params, dataset_name: str, repeat: int,
               scenarios: tuple[str, ...] = ALL,
               sizes: dict[str, tuple[int, int]] | None = None,
               *, tolerance: float = DEFAULT_COUNT_TOLERANCE,
               run_note: str = "",
               gate=None,
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
    ``geometry-scan`` and the write tier), per the inherited CSV
    contract — see that constant's docstring for the exact wording.

    ``gate`` is the run's load gate (``isolation.LoadGate``): consulted
    before each system's cell, it waits for a quiet node and records the
    load around the cell; a cell that proceeded above ``--max-load`` after
    the wait carries the ``busy`` token in ``notes``. Samples of one cell
    still run back to back, and cells are never interleaved.
    """
    sizes = sizes or {}
    rows: list[dict[str, str]] = []
    by_tag = {system.tag: system for system in systems}

    for scenario in scenarios:
        wanted = [by_tag[tag] for tag in systems_for(scenario) if tag in by_tag]
        if not wanted:
            continue
        for window, probe in variants(scenario, params):
            # The variant's own tag: for a window, suffixed `-approx` when
            # the row-fraction target was not reachable on this data, plus
            # the fraction it actually achieved — so a reader never has to
            # assume "1 %" meant 1 %. For the hit probe, the id asked for
            # and where in the canonical stream order it sits; for the miss,
            # its tag.
            variant_note = _variant_note(window, probe)
            measurements: dict[str, Measurement] = {}
            busy: set[str] = set()

            for system in wanted:
                if gate is not None and gate.before_cell(
                    " ".join(n for n in (run_note, scenario, variant_note, system.tag) if n)
                ):
                    busy.add(system.tag)
                try:
                    measurements[system.tag] = system.run(
                        scenario, params, repeat, window=window, probe=probe
                    )
                except ScenarioUnavailable as exc:
                    # A dataset property, not a system failure — kept
                    # distinct from `error:` so the paper can tell "nothing
                    # to ask" apart from "the system crashed".
                    measurements[system.tag] = _failed(f"skipped: {exc}")
                except Exception as exc:  # a system that cannot answer is a result
                    print(f"ERROR {scenario} {system.tag}: {type(exc).__name__}: {exc}",
                          file=sys.stderr, flush=True)
                    measurements[system.tag] = _failed(f"error: {type(exc).__name__}")
                finally:
                    if gate is not None:
                        gate.after_cell()

            answered = {
                tag: m for tag, m in measurements.items() if m.result_count is not None
            }
            # The two write tags run the same statements, so they are
            # genuine participants in the check rather than duplicates to
            # exclude.
            deviation, status = cross_check(
                {tag: m.result_count for tag, m in answered.items()}, tolerance
            )

            id_note, explained = _verify_identities(
                scenario, params, window, probe, wanted, answered, status, tolerance
            )
            if id_note:
                status = "id-mismatch"
            elif explained:
                status = "ok-deviation"
                deviation = f"{deviation} {explained}".strip()

            for tag, m in measurements.items():
                note = " ".join(
                    n for n in (run_note, variant_note, deviation, id_note or "",
                                "busy" if tag in busy else "") if n
                )
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
