---
title: Export CityGML LoD4 to CityJSON as lod 4.0 with a warning
date: 2026-10-08
status: accepted
summary: >-
  CityGML LoD4 geometry is kept as `geometry_lod4_0`; CityJSON export writes it with
  lod "4.0" and a warning, since CityJSON's free-form `lod` string can carry it
  even though no common CityJSON tool expects it.
tags: [citygml, export, lod]
related: []
supersedes: []
superseded_by: null
---

## What changed

The CityGML 2.0 reader maps LoD4 geometry to `geometry_lod4_0`, and CityGML export
writes it back as LoD4. CityJSON has no LoD4 — CityGML 3.0 folds it into LoD3 with
interiors — so export to CityJSON needed a rule. It writes the geometry with lod
`"4.0"`, and the CLI warns how many LoD4 geometries were written.

## Why

CityJSON's `lod` member is a string the specification does not restrict to a fixed
set, so `"4.0"` survives a round trip losslessly. A warning is enough to tell the
user that viewers and validators built for LoD 0–3 may ignore it.

### Alternatives considered

- **Drop LoD4 on CityJSON export, with a warning.** Rejected by the author: it loses
  data that CityGML export keeps.

## Consequences

A CityGML → CityParquet → CityJSON chain keeps LoD4, but a strict CityJSON
consumer may not render it. Whether the specification's CityJSON mapping page should
state this rule is an open question for the author.
