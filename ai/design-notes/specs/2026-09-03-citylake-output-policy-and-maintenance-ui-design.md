# A path policy for the API, and a UI for the operations that have none

**Date:** 2026-09-03
**Subject:** Giving CityLake's HTTP API a configured output root, and its web client screens for the seven operations that are currently reachable only by calling the API directly.

## 1. Why

Seven operations have no interface: `validate`, `reconcile`, `vacuum`, `compact`,
`merge`, `export` and `package` write. They were left out of the client's rebuild
deliberately — the scope was matched to what an end-to-end journey needed — and
nothing about them was decided against. This closes that gap.

Two of the seven cannot be given an interface honestly as things stand. `export`
and `package` write take a **server-side path from the caller**, which is one of
the four surfaces the API documents as trusted and does not constrain. Putting a
text box on a page does not create that exposure, but it does move it from
"an API caller could do this" to "a labelled field on an unauthenticated page",
which is a meaningful change in who reaches it and how easily.

So the policy comes first, and the interface follows it.

## 2. Where the policy belongs, and why not deeper

**At the HTTP boundary, in the handlers — not in the repository.**

The finding this answers is about an unauthenticated HTTP API being able to write
anywhere on the server's filesystem. A library consumer of `citylake` is already
running arbitrary code in the same process; constraining the paths it may pass
buys nothing and costs the ability to write a package to a temporary directory,
which is exactly what the crate's own tests legitimately do — piece A's real-data
round trip and the package suite both pass absolute temporary paths.

So `export_module_impl` and `write_package_impl` keep their current signatures and
keep accepting any path. `src/app/handlers/package.rs` resolves what a request
supplies against a configured root before calling them. The library stays a
library; the API gains a boundary.

## 3. The policy

**A new configuration field, `output_root`**, from `CITYLAKE_OUTPUT_ROOT`,
alongside the five the server already reads. It has **no default**.

**An unset root refuses both operations.** Not a fallback to today's behaviour: a
control that is off unless someone remembers to switch it on is not a control, and
this codebase has no users to break. The refusal says what to set and why, so an
operator meets a sentence rather than a mystery.

**A request supplies a path relative to the root.** An absolute path is refused. So
is any path that escapes, and the check is not textual: the server joins the
relative path to the root, canonicalises the result, and requires the canonical
form to remain under the canonical root. Textual rejection of `..` is not enough,
because a symlink placed inside the root can point anywhere and no amount of string
inspection sees it.

**A path whose target does not exist yet still has to be checked** — `package`
write names a directory it is about to create. Canonicalisation needs an existing
path, so the deepest existing ancestor is canonicalised and the remainder checked
against it. A parent that does not exist either is refused rather than created
blindly.

**Refusals are 400**, because the caller sent something the API will not accept —
distinct from the 422 that means the extension refused the caller's data.

## 4. What the client gains

All seven operations live on `DatasetDetailPage`, which is where a dataset's
identity already is. Nothing gets its own route.

**A maintenance section** holding the four operations that need no input:

- **Validate** — runs the checks and reveals the findings: check name, severity,
  table, object id, message. No findings is a stated result ("no problems found"),
  never a blank area, because those look identical and mean opposite things.
- **Reconcile** — re-derives what a structural edit invalidates. No confirmation:
  it loses nothing, and on an already-consistent package it is a no-op.
- **Vacuum** — reports how many unreferenced sidecar rows it reclaimed.
  Confirmation required: it deletes.
- **Compact** — reports files processed and files created. No confirmation.

**Merge**, in a dialog with a source picked from the existing dataset list, self
excluded. It states the preconditions before the user commits: object ids must be
unique across the whole destination, and the two CRSs must agree, or the extension
refuses the entire merge rather than partially applying it. Destructive to the
destination; confirmation required.

**Export** and **package write**, each in a dialog that shows the configured root
and takes a path relative to it. Export additionally takes the module and the
format. Package write reports the files it wrote, with their row counts and sizes —
information worth showing, since it is how a user learns the package is what they
expected.

Every operation surfaces its own errors where it was invoked, rather than in a
shared banner. A 400 from the path policy and a 422 from a refused merge mean
different things and belong next to the control that caused them.

## 5. Testing

**The policy gets Rust tests at the handler layer**, because that is where it
lives: a relative path resolving inside the root, an absolute path refused, a `..`
escape refused, a symlink escape refused, an unset root refusing, and a
not-yet-existing directory whose parent is inside the root accepted. The symlink
case is the one worth writing carefully — it is the case a textual check passes.

**The client gets end-to-end coverage** for the operations an automated run can
exercise honestly: validate on a clean dataset, reconcile, compact, and a merge
between two datasets created in the test. Export and package write are covered
through the policy's Rust tests plus one end-to-end case writing inside a root the
harness configures.

**Vacuum's positive path stays uncovered**, as it is today: no fixture produces an
orphaned sidecar row, and manufacturing one through the interface is not something
a user can do. Stated here so it is a known gap rather than an assumed pass.

## 6. Out of scope

No download endpoint. An export that streams the file back to the browser is
arguably what a person wants from the word "export", but it is a new API surface
rather than a UI change, it does not help package write — which produces a
directory — and it deserves its own piece of work.

No authentication. The path policy closes one of the four documented trust
surfaces at the HTTP boundary. The other three — `source_path` reads, the SQL
`filter` predicate, and attribute-update column names — are untouched, and the API
still has no authentication at all. This piece narrows one hole; it does not make
the API safe to expose.
