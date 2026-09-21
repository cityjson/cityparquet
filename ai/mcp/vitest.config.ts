import { defineConfig } from "vitest/config";

// `hookTimeout` covers `createEngine` in a `beforeAll`: against a fresh
// extension directory it downloads three extensions, and parallel suites
// doing that at once overran vitest's 10-second default.
export default defineConfig({
  test: { include: ["test/**/*.test.ts"], testTimeout: 120_000, hookTimeout: 120_000 },
});
