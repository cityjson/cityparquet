# CityParquet documentation

The documentation site for **CityParquet**, a columnar Parquet encoding for 3D
city models. Built with [Blume](https://useblume.dev) (markdown on Astro);
[Vite+](https://voidzero.dev) (`vp`) handles linting and formatting.

## Commands

Run these from this `documents/` folder. Requires Node 22.12+ and pnpm.

| Command | What it does |
| --- | --- |
| `pnpm install` | Install dependencies |
| `pnpm dev` | Start the dev server with hot reload |
| `pnpm build` | Build the static site to `dist/` |
| `pnpm preview` | Serve the production build locally |
| `pnpm lint` | Lint (`vp` / oxlint) |
| `pnpm format` | Format in place (`vp` / oxfmt) |
| `pnpm format:check` | Check formatting without writing |

## Writing content

Pages live under `docs/` as MDX. The sidebar is inferred from the file tree and
refined per folder with `meta.ts`; site-wide settings are in `blume.config.ts`.

## Deploying

`DOCS_BASE_PATH` and `DOCS_SITE_URL` are read from the environment, so moving the
site to its own public repository needs no content changes:

```bash
DOCS_BASE_PATH=/cityparquet DOCS_SITE_URL=https://cityjson.github.io pnpm build
```

Output is written to `dist/` — deploy that folder to any static host.
