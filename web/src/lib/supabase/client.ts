import { createBrowserClient } from "@supabase/ssr";
import type { SupabaseClient } from "@supabase/supabase-js";

const url = import.meta.env.VITE_SUPABASE_URL;
const publishableKey = import.meta.env.VITE_SUPABASE_PUBLISHABLE_KEY;

/** True only when both the project URL and the publishable key are wired in. */
export const isSupabaseConfigured: boolean = Boolean(url && publishableKey);

// Falling back to placeholder values keeps the bundle importable when env is
// missing — pages that need auth surface a friendly message via
// `isSupabaseConfigured` instead of the whole app crashing on first import.
const SUPABASE_PLACEHOLDER_URL = "https://placeholder.supabase.invalid";
const SUPABASE_PLACEHOLDER_KEY = "placeholder-anon-key";

/**
 * Create a Supabase browser client.
 *
 * The shadcn registry intentionally ships a factory (rather than a singleton)
 * so the same module can be imported in SSR contexts. We don't have SSR here,
 * so [`@/lib/supabase`](../supabase.ts) calls this once and re-exports a
 * singleton that the rest of the app uses.
 *
 * Auth options:
 * - `flowType: 'pkce'` is the modern OAuth flow.
 * - `detectSessionInUrl: false` because we exchange the code explicitly in
 *   `pages/AuthCallbackPage.tsx`. Letting supabase-js auto-handle the URL
 *   would race the callback page and consume the code before we can show
 *   any error.
 */
export function createClient(): SupabaseClient {
  return createBrowserClient(
    url || SUPABASE_PLACEHOLDER_URL,
    publishableKey || SUPABASE_PLACEHOLDER_KEY,
    {
      auth: {
        flowType: "pkce",
        detectSessionInUrl: false,
      },
    },
  );
}
