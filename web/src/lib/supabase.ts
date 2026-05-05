import { createClient } from "@supabase/supabase-js";

const url = import.meta.env.VITE_SUPABASE_URL;
const anonKey = import.meta.env.VITE_SUPABASE_ANON_KEY;

if (!url || !anonKey) {
  // Soft warning during dev; the login page will surface a friendlier message
  // when an action is attempted.
  // eslint-disable-next-line no-console
  console.warn(
    "Supabase env not set: VITE_SUPABASE_URL / VITE_SUPABASE_ANON_KEY. Auth will fail until configured.",
  );
}

export const supabase = createClient(url ?? "", anonKey ?? "");
