import { createClient, isSupabaseConfigured } from "@/lib/supabase/client";

if (!isSupabaseConfigured) {
  // eslint-disable-next-line no-console
  console.warn(
    "Supabase env not set: VITE_SUPABASE_URL / VITE_SUPABASE_PUBLISHABLE_KEY. Auth will fail until configured.",
  );
}

/** Singleton browser client used across the app. */
export const supabase = createClient();
export { isSupabaseConfigured };
