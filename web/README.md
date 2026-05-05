# CityLake Web

A React + Vite + TypeScript + Tailwind + Shadcn UI scaffold for the CityLake web app.
This directory ships only the skeleton — see [`../design/PLAN.md`](../design/PLAN.md)
for the full architecture and feature roadmap.

## Stack

- React 19 + Vite 6
- TypeScript (strict)
- Tailwind CSS 3 + Shadcn UI conventions (CSS variables in `src/index.css`)
- TanStack Query for server state
- React Router for client-side routing
- Supabase (auth only — data lives in CityLake)

## Setup

```bash
cd web
npm install      # or pnpm install
cp .env.example .env.local
# fill in VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY
npm run dev
```

In dev, requests to `/api/...` are proxied to `http://127.0.0.1:3000` (the local
CityLake server). Override the prefix with `VITE_API_BASE_URL` for staging/prod.

## Layout

```
src/
├── App.tsx              # Authenticated shell (placeholder)
├── main.tsx             # Entry: providers + routes
├── index.css            # Tailwind base + Shadcn CSS variables
├── lib/
│   ├── api.ts           # Typed CityLake client (uses Supabase JWT)
│   ├── supabase.ts      # Supabase client (auth only)
│   └── utils.ts         # `cn()` helper
├── pages/
│   └── LoginPage.tsx    # Magic-link login
└── vite-env.d.ts
```

## Implementation status

| Phase | Status |
| --- | --- |
| 0 — scaffold | done (this commit) |
| 1 — auth flow | not started |
| 2 — list / detail (read) | not started |
| 3 — upload | not started |
| 4 — CRUD | not started |
| 5 — server-side gaps (`GET /tables`, JWT middleware) | not started |

See `design/PLAN.md` for the per-phase scope.

## Related changes needed in the Rust crate

The web app cannot fully ship without these CityLake server additions
(intentionally out of scope for the scaffold commit):

- `GET /tables` endpoint that returns the list of tables in the
  `citylake` catalog — needed to render the dataset index.
- JWT validation middleware that accepts Supabase access tokens.
- Tightened CORS rules (currently permissive in `src/app/middleware/`).
