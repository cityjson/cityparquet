# CityLake Web — Architecture & Feature Plan

## Goals

A web UI for non-technical users to upload CityJSON datasets to CityLake,
browse the LOD-suffixed tables, and CRUD individual CityObjects, behind
authentication.

## Stack

| Concern | Choice | Notes |
| --- | --- | --- |
| Framework | React 19 + Vite + TypeScript | fast dev loop |
| Styling | Tailwind CSS + Shadcn UI | per `tasks.md` |
| State / data | TanStack Query | cache + retries on top of fetch |
| Routing | TanStack Router or React Router | TBD; pick whichever Shadcn templates target |
| Forms | react-hook-form + zod | typed validation for upload/CRUD forms |
| Auth | Supabase Auth (email + magic link) | per `tasks.md`, no role system needed |
| Backend | CityLake HTTP API (Rust) | hits `/tables/...`; no app-side DB |

## Architecture

```
┌──────────────┐       JWT        ┌──────────────────┐
│ React + Vite │ ─────────────▶  │ Supabase Auth     │
│ (Shadcn UI)  │ ◀─────────────  │ (issues JWT)      │
└─────┬────────┘                 └──────────────────┘
      │  Authorization: Bearer <jwt>
      ▼
┌──────────────────────────────┐    SQL    ┌────────────┐
│ CityLake server (axum, Rust) │ ───────▶ │ DuckDB +    │
│   /tables/...                │           │  DuckLake   │
└──────────────────────────────┘           └────────────┘
```

The web app **does not** use Supabase Postgres for data. Supabase is auth-only.
All CityJSON data lives in CityLake's DuckLake catalog (Parquet under the hood).

The CityLake server gains a thin auth middleware that validates the Supabase JWT
(via Supabase's JWKS) before routing to the existing handlers. That middleware is
out of scope for the initial scaffold — it's the first piece of the next session.

## Pages / Routes

| Route | Purpose |
| --- | --- |
| `/` | Public landing → redirects to `/login` or `/datasets` based on auth state |
| `/login` | Supabase email/magic-link form |
| `/datasets` | List of datasets (= `base_name`s grouped from `cityjson_metadata`) |
| `/datasets/:base` | Detail view: per-LOD tables, row counts, source metadata |
| `/datasets/:base/lod/:lod` | Browse CityObjects in a specific LOD table; pagination + filter |
| `/datasets/:base/lod/:lod/:id` | Edit / delete a single CityObject |
| `/upload` | Upload CityJSON file (multipart) to create a new dataset |

## Feature breakdown (from `tasks.md`)

1. **Upload CityJSON files**
   - Page `/upload` → multipart POST to `/tables/{base}/upload?lod=&base_name=`
   - Form fields: file picker, optional `lod`, optional override base name
   - Show progress + final list of created tables

2. **List of tables / datasets**
   - Page `/datasets` → fetch `/tables/cityjson_metadata/objects` for the
     authoritative list of ingested datasets, then for each base derive the
     LOD tables by listing `information_schema.tables` (needs a new
     read-only HTTP endpoint, see "Server gaps" below)
   - Drill-down to `/datasets/:base/lod/:lod` for row-level browsing

3. **CRUD on CityObjects**
   - Read: `GET /tables/{table}/objects?filter=&limit=&offset=`
   - Update: `PUT /tables/{table}/objects/{id}` with raw CityJSON snippet
   - Delete: `DELETE /tables/{table}/objects/{id}`
   - No create-single-object endpoint exists today; the web app uses
     bulk insert (`POST /tables/{base}/objects/upload`) for new objects.

4. **Auth**
   - Supabase Auth on the React side; CityLake server gains JWT validation
     middleware. No roles — any authenticated user can do everything.

## Server gaps (work for a future session)

These are not needed for the scaffold but the web app cannot ship without them:

- `GET /tables` — list all tables in the citylake catalog (currently no
  endpoint exposes this; web app needs it to render the dataset index).
- JWT auth middleware in the axum router that validates Supabase tokens.
- CORS rules tightened to the deployed web origin (today the layer is
  permissive).
- Multi-LOD round-trip export (already deferred — see `tasks.md`).

## Implementation phases

| Phase | Scope | Status |
| --- | --- | --- |
| 0 — scaffold | Vite + Shadcn skeleton, README, env template, this PLAN | done |
| 1 — auth | Supabase client wiring, login page, JWT propagation in fetch helper | done |
| 2 — list / detail | `/datasets`, `/datasets/:base`, `/tables/:tableName` (read-only) | done |
| 3 — upload | `/upload` page with drag-and-drop multipart | done |
| 4 — CRUD | edit dialog (CityJSON textarea) + delete confirm | done |
| 5 — server-side | `GET /tables` endpoint **(done)** + JWT middleware in CityLake | partial |

Phase 5's JWT middleware is the only remaining piece — the web app currently
authenticates the user client-side and propagates the access token in a
`Bearer` header, but the Rust server does not yet validate it.
