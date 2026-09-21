# Hosted deployment

The Cloudflare Worker that runs the MCP server's image
([`../Dockerfile`](../Dockerfile)) as a Cloudflare Container and routes
`/mcp` and `/health` to it. `.github/workflows/mcp-deploy.yml` deploys it from
`main`, or from any branch when run by hand, then runs [`smoke.mjs`](smoke.mjs)
against the deployment.

What it sets, in [`src/index.ts`](src/index.ts):

| Setting | Value | Why |
| --- | --- | --- |
| Egress | `enableInternet = false`, `allowedHosts` = the three `open3d.city` data hosts | The public `query` tool can name any URL; the platform, not DuckDB, decides what it can reach |
| `deniedHosts` | private, loopback and link-local ranges | Defence in depth only |
| Instances | 2, `standard-1` (½ vCPU, 4 GiB) | Containers do not autoscale; each instance runs 2 engines |
| `sleepAfter` | 10 minutes | An instance stops when idle; the next request pays a cold start of a few seconds |

Deploying needs the Workers Paid plan and an API token with **Workers
Scripts: Edit** and **Containers: Edit**, held as the repository secrets
`CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`. By hand:

```sh
pnpm install
pnpm exec wrangler deploy          # builds ../Dockerfile with Docker
node smoke.mjs https://cityparquet-mcp.<your-subdomain>.workers.dev
```

The image has to build for `linux/amd64`.
