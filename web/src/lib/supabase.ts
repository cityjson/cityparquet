import { createClient, type SupabaseClient } from "@supabase/supabase-js";

const url = import.meta.env.VITE_SUPABASE_URL;
const anonKey = import.meta.env.VITE_SUPABASE_ANON_KEY;

export const isSupabaseConfigured = Boolean(url && anonKey);

if (!isSupabaseConfigured) {
  // Soft warning during dev; pages that need auth surface a clearer message
  // when an action is attempted. We still construct a client below — using
  // placeholder values — so the rest of the bundle can import this module
  // without a top-level throw on first render.
  // eslint-disable-next-line no-console
  console.warn(
    "Supabase env not set: VITE_SUPABASE_URL / VITE_SUPABASE_ANON_KEY. Auth will fail until configured.",
  );
}

// `createClient` validates the URL syntactically, so blank strings throw.
// Falling back to a stub URL keeps the module importable; any subsequent
// network call surfaces an obvious error path-traceable to the missing env.
const SUPABASE_PLACEHOLDER_URL = "https://placeholder.supabase.invalid";
const SUPABASE_PLACEHOLDER_KEY = "placeholder-anon-key";

export const supabase: SupabaseClient = createClient(
  url || SUPABASE_PLACEHOLDER_URL,
  anonKey || SUPABASE_PLACEHOLDER_KEY,
  {
    auth: {
      flowType: "pkce",
      // We handle the OAuth callback explicitly in /auth/callback so failures
      // surface as a real error UI. Letting supabase-js auto-process the URL
      // on import would race with our explicit exchange and consume the code
      // before we can see what happened to it.
      detectSessionInUrl: false,
    },
  },
);
