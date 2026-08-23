import { defineConfig } from "blume";

// Base path and site URL are read from the environment so the later split to a
// public `cityparquet-docs` repo (GitHub Pages) needs no content changes — see
// docs/superpowers/specs/2026-07-17-cityparquet-docs-site-design.md.
//
//   DOCS_BASE_PATH=/cityparquet-docs   # GitHub Pages project-site subpath
//   DOCS_SITE_URL=https://hideba.github.io
const base = process.env.DOCS_BASE_PATH || undefined;
const site = process.env.DOCS_SITE_URL || undefined;

export default defineConfig({
  title: "CityParquet",
  description:
    "A columnar Parquet encoding for 3D city models — specification, design rationale, tutorials, and benchmarks.",
  // The lockup already contains the "CityParquet" wordmark, so `text: ""`
  // suppresses the site title beside it — otherwise the name renders twice.
  logo: { image: "/brand/cityparquet-lockup.svg", text: "" },

  content: {
    root: "docs",
  },

  // Points the header repo link and Edit-on-GitHub actions at the public docs
  // repo the site is published from. Harmless while it still lives in the paper
  // repo; edit links resolve once the split lands.
  github: {
    owner: "HideBa",
    repo: "cityparquet-docs",
    branch: "main",
  },

  banner: {
    content: "CityParquet is v0.1.0-draft — the specification is still changing.",
    id: "draft-0-1-0",
    dismissible: true,
  },

  // Matches cityjson.org's look (the "just-the-docs" Jekyll theme): its
  // signature link/brand purple (#7253ed), the same subtle ~4px corner
  // rounding used throughout its nav/buttons/code blocks, and Roboto — the
  // font its `system-ui` stack resolves to on most non-Apple platforms, and
  // the one named font in that stack Blume's curated set actually has.
  theme: {
    accent: "#7253ed",
    radius: "sm",
    mode: "system",
    fonts: {
      body: "roboto",
      display: "roboto",
      mono: "roboto-mono",
    },
  },

  navigation: {
    sidebar: {
      display: "group",
    },
  },

  search: {
    provider: "orama",
  },

  markdown: {
    imageZoom: true,
    code: {
      icons: true,
      wrap: false,
    },
  },

  ai: {
    llmsTxt: true,
  },

  seo: {
    og: { enabled: true },
    sitemap: true,
    robots: true,
    structuredData: true,
  },

  lastModified: true,

  deployment: {
    output: "static",
    ...(site ? { site } : {}),
    ...(base ? { base } : {}),
  },
});
