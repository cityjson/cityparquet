import { createClient as createSupabaseClient } from "@supabase/supabase-js";
import type { SupabaseClient } from "@supabase/supabase-js";

const url = import.meta.env.VITE_SUPABASE_URL;
const publishableKey = import.meta.env.VITE_SUPABASE_PUBLISHABLE_KEY;

/** True only when both the project URL and the publishable key are wired in. */
export const isSupabaseConfigured: boolean = Boolean(url && publishableKey);

/**
 * Identify whether a Supabase API key is the public/anon key or the secret
 * server-only key. Supports both formats Supabase has shipped:
 *
 *   * Legacy JWT keys — payload `role` claim is `anon` or `service_role`.
 *   * 2025+ static keys — string prefix `sb_publishable_` or `sb_secret_`.
 *
 * Returns `null` when the value is unrecognised.
 */
export function detectKeyRole(key: string | undefined): "anon" | "service_role" | null {
  if (!key) return null;

  if (key.startsWith("sb_publishable_")) return "anon";
  if (key.startsWith("sb_secret_")) return "service_role";

  const parts = key.split(".");
  if (parts.length !== 3) return null;
  try {
    const payload = JSON.parse(atob(parts[1].replace(/-/g, "+").replace(/_/g, "/")));
    if (payload.role === "anon" || payload.role === "service_role") {
      return payload.role;
    }
    return null;
  } catch {
    return null;
  }
}

/**
 * `true` if the configured key is the secret server-only key — using this in
 * the browser is a security incident *and* breaks auth (Supabase returns
 * "Forbidden use of secret API key in browser" on every request).
 */
export const isServiceRoleKey: boolean = detectKeyRole(publishableKey) === "service_role";

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
