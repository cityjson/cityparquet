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
├── App.tsx                       # Protected shell + nested routes
├── main.tsx                      # Entry: providers + auth provider
├── index.css                     # Tailwind base + Shadcn CSS variables
├── auth/
│   ├── AuthContext.tsx           # Supabase session listener + useAuth
│   └── ProtectedRoute.tsx        # Redirects unauth'd users to /login
├── components/
│   ├── AppShell.tsx              # Sidebar + outlet layout
│   └── ui/                       # Shadcn primitives
│       ├── alert-dialog.tsx
│       ├── button.tsx
│       ├── card.tsx
│       ├── dialog.tsx
│       ├── input.tsx
│       ├── label.tsx
│       ├── skeleton.tsx
│       ├── table.tsx
│       └── textarea.tsx
├── lib/
│   ├── api.ts                    # Typed CityLake client (Supabase JWT)
│   ├── supabase.ts               # Supabase client (auth only)
│   └── utils.ts                  # `cn()` helper
├── pages/
│   ├── DatasetsPage.tsx          # /datasets — list of base names
│   ├── DatasetDetailPage.tsx     # /datasets/:base — LOD tables + metadata
│   ├── LodTablePage.tsx          # /tables/:tableName — browse + edit + delete
│   ├── LoginPage.tsx             # /login — magic-link auth
│   └── UploadPage.tsx            # /upload — multipart create_table
└── vite-env.d.ts
```

## Implementation status

| Phase | Status |
| --- | --- |
| 0 — scaffold | done |
| 1 — auth flow | done (Supabase magic-link, AuthProvider, ProtectedRoute) |
| 2 — list / detail (read) | done (DatasetsPage, DatasetDetailPage, LodTablePage browse) |
| 3 — upload | done (UploadPage with drag-and-drop) |
| 4 — CRUD | done (edit dialog, delete confirm) |
| 5 — server-side JWT auth | not started |

## Related changes still needed in the Rust crate

- `GET /tables` endpoint — **done** (this commit added it).
- JWT validation middleware that accepts Supabase access tokens.
- Tightened CORS rules (currently permissive in `src/app/middleware/`).
