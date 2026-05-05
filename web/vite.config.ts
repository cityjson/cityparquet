import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig(() => {
  // Read directly from `process.env` so vars injected upstream by dotenvx are
  // visible. Vite's own `loadEnv()` only scans .env files in the project dir
  // and would miss the workspace-root .env that dotenvx decrypts.
  //
  // The Supabase project URL and anon (publishable) key are public by design;
  // aliasing across naming conventions lets developers keep a single
  // workspace-root .env without duplicating values:
  //
  //   * URL:  VITE_SUPABASE_URL  ←  SUPABASE_URL
  //   * Key:  VITE_SUPABASE_PUBLISHABLE_KEY  ←  SUPABASE_PUBLISHABLE_KEY
  //                                            SUPABASE_ANON_KEY
  //                                            SUPABASE_KEY
  //                                            VITE_SUPABASE_ANON_KEY
  //
  // Anything else in the .env stays gated behind Vite's default VITE_ prefix.
  const supabaseUrl =
    process.env.VITE_SUPABASE_URL || process.env.SUPABASE_URL || "";
  const supabasePublishableKey =
    process.env.VITE_SUPABASE_PUBLISHABLE_KEY ||
    process.env.SUPABASE_PUBLISHABLE_KEY ||
    process.env.VITE_SUPABASE_ANON_KEY ||
    process.env.SUPABASE_ANON_KEY ||
    process.env.SUPABASE_KEY ||
    "";

  if (!supabaseUrl || !supabasePublishableKey) {
    // eslint-disable-next-line no-console
    console.warn(
      `[vite.config] Supabase env not picked up. process.env keys seen: ${Object.keys(
        process.env,
      )
        .filter((k) => k.startsWith("SUPABASE") || k.startsWith("VITE_"))
        .join(", ") || "(none)"}`,
    );
  }

  return {
    plugins: [react()],
    resolve: {
      alias: {
        "@": path.resolve(__dirname, "./src"),
      },
    },
    define: {
      "import.meta.env.VITE_SUPABASE_URL": JSON.stringify(supabaseUrl),
      "import.meta.env.VITE_SUPABASE_PUBLISHABLE_KEY":
        JSON.stringify(supabasePublishableKey),
    },
    server: {
      port: 5173,
      proxy: {
        // Forward API calls to the local CityLake server during dev so we don't
        // need CORS toggles. Override with VITE_API_BASE_URL when deployed.
        "/api": {
          target: "http://127.0.0.1:3000",
          changeOrigin: true,
          rewrite: (p) => p.replace(/^\/api/, ""),
        },
      },
    },
  };
});
