import { createClient as createSupabaseClient } from "@supabase/supabase-js";
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
 * The shadcn `@supabase/supabase-client-react-router` registry installs
 * `@supabase/ssr`'s `createBrowserClient`, which stores the PKCE code
 * verifier in cookies so it can be read by a server-side counterpart in SSR
 * frameworks (Next.js, SvelteKit, React Router's framework mode). CityLake's
 * web app is a pure SPA driven by Vite, so we use `@supabase/supabase-js`'s
 * `createClient` directly — it persists the PKCE verifier in localStorage,
 * which round-trips reliably across the OAuth redirect without requiring a
 * server. `@supabase/ssr` stays installed (and the registry stays registered
 * in `components.json`) so future Supabase shadcn blocks Just Work.
 *
 * Auth options:
 * - `flowType: 'pkce'` is the modern OAuth flow.
 * - `detectSessionInUrl: false` because we exchange the code explicitly in
 *   `pages/AuthCallbackPage.tsx`. Letting supabase-js auto-handle the URL
 *   would race the callback page and consume the code before we can show
 *   any error.
 */
export function createClient(): SupabaseClient {
  return createSupabaseClient(
    url || SUPABASE_PLACEHOLDER_URL,
    publishableKey || SUPABASE_PLACEHOLDER_KEY,
    {
      auth: {
        flowType: "pkce",
        detectSessionInUrl: false,
        persistSession: true,
        autoRefreshToken: true,
      },
    },
  );
}
