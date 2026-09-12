import type { Session } from "@supabase/supabase-js";
import { createContext, useContext, useEffect, useMemo, useState, type ReactNode } from "react";

import { supabase } from "@/lib/supabase";

interface AuthState {
  session: Session | null;
  loading: boolean;
  signOut: () => Promise<void>;
}

const AuthContext = createContext<AuthState | null>(null);

/**
 * A stand-in session for automated interface tests.
 *
 * Both conditions are required, and the first is the one that matters:
 * Vite replaces `import.meta.env.DEV` with the literal `false` in a
 * production build, so this branch is constant-folded away and the built
 * artefact does not contain it. The environment variable alone would be a
 * switch anybody could flip in production; paired with DEV it can only ever
 * be flipped in a development server.
 */
const E2E_SESSION_ACTIVE = import.meta.env.DEV && import.meta.env.VITE_E2E_AUTH_BYPASS === "1";

/**
 * Deliberately minimal — not a faithful `Session`. It only needs to satisfy
 * `ProtectedRoute`'s truthiness check and give any reader of
 * `session.access_token` a value to find, should one look. `src/lib/api.ts`
 * is not such a reader: it calls `supabase.auth.getSession()` directly
 * rather than going through this context, so this token is never actually
 * sent anywhere.
 */
const E2E_SESSION = {
  access_token: "e2e-bypass-token",
  token_type: "bearer",
  user: { id: "e2e-bypass-user" },
} as Session;

export function AuthProvider({ children }: { children: ReactNode }) {
  const [session, setSession] = useState<Session | null>(E2E_SESSION_ACTIVE ? E2E_SESSION : null);
  const [loading, setLoading] = useState(!E2E_SESSION_ACTIVE);

  useEffect(() => {
    if (E2E_SESSION_ACTIVE) {
      // No Supabase project needs to be reachable for an automated run.
      return;
    }

    let active = true;

    supabase.auth.getSession().then(({ data }) => {
      if (!active) return;
      setSession(data.session);
      setLoading(false);
    });

    const { data: subscription } = supabase.auth.onAuthStateChange((_event, next) => {
      setSession(next);
    });

    return () => {
      active = false;
      subscription.subscription.unsubscribe();
    };
  }, []);

  const value = useMemo<AuthState>(
    () => ({
      session,
      loading,
      signOut: async () => {
        if (E2E_SESSION_ACTIVE) {
          // Nothing to sign out of.
          return;
        }
        await supabase.auth.signOut();
      },
    }),
    [session, loading],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth(): AuthState {
  const ctx = useContext(AuthContext);
  if (!ctx) {
    throw new Error("useAuth must be used inside <AuthProvider>");
  }
  return ctx;
}
