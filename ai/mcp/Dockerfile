# The hosted CityParquet MCP server. Built from ai/mcp alone: the corpus is
# committed, so no submodule is needed. .github/workflows/mcp-deploy.yml
# deploys it to Cloud Run.

FROM node:24-slim AS build
WORKDIR /app
RUN corepack enable && corepack prepare pnpm@10 --activate
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.json tsconfig.build.json ./
RUN pnpm install --frozen-lockfile
COPY src ./src
RUN pnpm build \
 && pnpm install --frozen-lockfile --prod --config.confirmModulesPurge=false

FROM node:24-slim
ENV NODE_ENV=production \
    PORT=8080 \
    CITYPARQUET_MCP_EXTENSION_DIR=/opt/duckdb-extensions
WORKDIR /app
# httpfs verifies TLS against the system CA bundle, which the slim image
# lacks: without it every https:// read fails, the bake included.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /app/package.json ./
COPY --from=build /app/node_modules ./node_modules
COPY --from=build /app/dist ./dist
COPY corpus ./corpus
# The extensions are part of the image, never fetched at run time: a
# sandboxed engine's network goes through the egress proxy, which admits the
# data hosts and nothing else.
RUN node dist/bake.js && chmod -R a+rX /opt/duckdb-extensions
USER node
EXPOSE 8080
CMD ["node", "dist/http.js"]
