# CityLake Web

The CityLake web UI — upload a CityJSON-family file into a dataset, browse its
module tables, and CRUD individual CityObjects, all behind Supabase auth.

## Stack

- React 19 + Vite 8 (driven by **[Vite+]**)
- TypeScript (strict)
- Tailwind CSS 3, themed against the CityLake design system
  (`src/styles/citylake.css` — Lake / Ink / Paper / Roof / Moss / Sun palette,
  IBM Plex type ramp, 4 px-grid spacing)
- TanStack Query for server state
- React Router for client-side routing
- Supabase (auth only — data lives in CityLake), wired through the official
  [`@supabase/supabase-client-react-router`][supabase-client] shadcn registry
  (drops `@supabase/ssr` + `src/lib/supabase/client.ts`)

[supabase-client]: https://supabase.com/ui/r/supabase-client-react-router
[Vite+]: https://viteplus.dev/

## Toolchain

This package is built and tested with **Vite+** (`vp`), which bundles dev/build
(Vite + Rolldown), check (oxlint + oxfmt + tsgo), and test (Vitest) into a
single CLI. ESLint and Prettier are intentionally absent — `vp check` covers
both jobs at once.

Install Vite+ once per machine:

```bash
# macOS / Linux
curl -fsSL https://vite.plus | bash

# Windows (PowerShell)
irm https://vite.plus/ps1 | iex
```

Then in this directory:

```bash
npm install               # resolve runtime deps
cp .env.example .env.local
# fill in VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY
vp dev                    # dev server with HMR
vp build                  # production bundle
vp check                  # lint + format + typecheck
vp test                   # vitest
```

In dev, requests to `/api/...` are proxied to `http://127.0.0.1:3000` (the
local CityLake server). Override the prefix with `VITE_API_BASE_URL` for
staging/prod.

## Design system

The visual language follows the CityLake design system: drafting-paper warm
backgrounds, IBM Plex Sans / Mono type, Lake-teal accent, sharp corners
(0–8 px radii), no purple/glass/gradients. Colour tokens, type scale, spacing,
shadows and semantic styles are CSS variables in
[`src/styles/citylake.css`](./src/styles/citylake.css), mirrored into Tailwind
via the brand families in [`tailwind.config.ts`](./tailwind.config.ts).

Logos and the CRS grid overlay live in [`src/assets/`](./src/assets/).

## Layout

```
src/
├── App.tsx                       # Protected shell + nested routes
├── main.tsx                      # Entry: providers + auth provider
├── index.css                     # Imports citylake.css + Tailwind layers
├── styles/citylake.css           # Design tokens (palette, type, spacing, shadows)
├── assets/                       # logo-mark, logo-wordmark, grid-overlay, cityjson-glyph
├── auth/
│   ├── AuthContext.tsx           # Supabase session listener + useAuth
│   └── ProtectedRoute.tsx        # Redirects unauth'd users to /login
├── components/
│   ├── AppShell.tsx              # Sidebar + topbar + outlet
│   ├── Eyebrow.tsx               # Uppercased Plex-Mono labels
│   ├── StatusDot.tsx             # ok/warn/error dot
│   ├── Tag.tsx                   # Mono pill-or-square tag (READY, EPSG:7415, …)
│   └── ui/                       # shadcn primitives, restyled to the design
├── lib/
│   ├── api.ts                    # Typed CityLake client (Supabase JWT)
│   ├── supabase.ts               # Singleton browser client (auth only)
│   ├── supabase/client.ts        # @supabase/ssr factory — shadcn-installed
│   └── utils.ts                  # cn() helper
├── pages/
│   ├── DatasetsPage.tsx          # /datasets — dataset name card grid
│   ├── DatasetDetailPage.tsx     # /datasets/:ds — module list (role, rows) + CRS
│   ├── ModulePage.tsx            # /datasets/:ds/modules/:module — browse + delete
│   ├── LoginPage.tsx             # /login — magic-link auth
│   └── UploadPage.tsx            # /upload — multipart dataset upload
└── vite-env.d.ts
```

## Implementation status

| Phase                    | Status                                                   |
| ------------------------ | -------------------------------------------------------- |
| 0 — scaffold             | done                                                     |
| 1 — auth flow            | done (Supabase magic-link, AuthProvider, ProtectedRoute) |
| 2 — list / detail (read) | done                                                     |
| 3 — upload               | done                                                     |
| 4 — CRUD                 | done                                                     |
| 5 — server-side JWT auth | not started                                              |

## Related changes in the Rust crate

- Dataset and module endpoints (`GET /datasets`, `GET /datasets/{ds}`,
  `GET /datasets/{ds}/modules/{module}/objects`, …) — done.
- JWT validation middleware that accepts Supabase access tokens — pending.
- Scoped CORS — pending.
