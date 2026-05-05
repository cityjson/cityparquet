import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig(() => {
  // Read directly from `process.env` so vars injected upstream by dotenvx are
  // visible. Vite's own `loadEnv()` only scans .env files in the project dir
  // and would miss the workspace-root .env that dotenvx decrypts.
  //
  // The Supabase project URL and anon key are public by design; aliasing both
  // naming conventions here means developers can use either `SUPABASE_*` or
  // `VITE_SUPABASE_*` in their .env without duplicating values. Anything else
  // stays gated behind Vite's default VITE_ prefix.
  const supabaseUrl =
    process.env.VITE_SUPABASE_URL || process.env.SUPABASE_URL || "";
  const supabaseAnonKey =
    process.env.VITE_SUPABASE_ANON_KEY ||
    process.env.SUPABASE_ANON_KEY ||
    process.env.SUPABASE_KEY ||
    "";

  if (!supabaseUrl || !supabaseAnonKey) {
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
      "import.meta.env.VITE_SUPABASE_ANON_KEY": JSON.stringify(supabaseAnonKey),
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
