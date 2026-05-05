import { useEffect, useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";

import { Eyebrow } from "@/components/Eyebrow";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { isServiceRoleKey } from "@/lib/supabase/client";
import { supabase } from "@/lib/supabase";

/**
 * /auth/callback — explicit OAuth landing page.
 *
 * supabase-js can normally consume the OAuth response from any URL via
 * `detectSessionInUrl`, but routing it through a known page makes the success
 * and failure cases easy to surface in the UI: error params from Supabase end
 * up here as `?error=...&error_description=...`, and the PKCE code exchange's
 * own errors are awaited explicitly so they don't get lost in console noise.
 */
interface StorageReport {
  origin: string;
  pathname: string;
  localStorageKeys: string[];
  sessionStorageKeys: string[];
  cookieKeys: string[];
}

function snapshotStorage(): StorageReport {
  const lsKeys: string[] = [];
  for (let i = 0; i < window.localStorage.length; i++) {
    const k = window.localStorage.key(i);
    if (k) lsKeys.push(k);
  }
  const ssKeys: string[] = [];
  for (let i = 0; i < window.sessionStorage.length; i++) {
    const k = window.sessionStorage.key(i);
    if (k) ssKeys.push(k);
  }
  const cookieKeys = document.cookie
    ? document.cookie.split(";").map((c) => c.trim().split("=")[0]).filter(Boolean)
    : [];
  return {
    origin: window.location.origin,
    pathname: window.location.pathname,
    localStorageKeys: lsKeys.sort(),
    sessionStorageKeys: ssKeys.sort(),
    cookieKeys: cookieKeys.sort(),
  };
}

export default function AuthCallbackPage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const [error, setError] = useState<string | null>(null);
  const [diagnostics, setDiagnostics] = useState<StorageReport | null>(null);

  useEffect(() => {
    const errorParam =
      searchParams.get("error_description") || searchParams.get("error");
    if (errorParam) {
      setError(decodeURIComponent(errorParam));
      setDiagnostics(snapshotStorage());
      return;
    }

    const code = searchParams.get("code");
    const next = searchParams.get("next") || "/datasets";

    let cancelled = false;

    (async () => {
      // PKCE flow: exchange the code for a session. supabase-js will also
      // attempt this implicitly, but we await it here so we can show the
      // error if it returns one.
      if (code) {
        const { error: exchangeError } =
          await supabase.auth.exchangeCodeForSession(code);
        if (cancelled) return;

        if (exchangeError) {
          // Some "exchange failed" cases are recoverable: the user already
          // has a valid session in storage from a prior successful flow, but
          // a stale `?code=` got replayed (e.g. browser back-button or a
          // duplicate tab). If so, treat the existing session as authoritative
          // and continue silently rather than blocking on a redundant error.
          const { data } = await supabase.auth.getSession();
          if (cancelled) return;
          if (data.session) {
            navigate(next, { replace: true });
            return;
          }

          setError(`Code exchange failed: ${exchangeError.message}`);
          setDiagnostics(snapshotStorage());
          return;
        }
      } else {
        // Implicit flow: token sat in the URL hash. supabase-js handles that
        // on import; nothing to do here besides confirm.
        const { error: sessionError } = await supabase.auth.getSession();
        if (cancelled) return;
        if (sessionError) {
          setError(sessionError.message);
          setDiagnostics(snapshotStorage());
          return;
        }
      }

      navigate(next, { replace: true });
    })();

    return () => {
      cancelled = true;
    };
  }, [searchParams, navigate]);

  if (!error) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-paper-50 font-mono text-[12px] text-ink-500">
        Completing sign-in…
      </div>
    );
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-paper-50 p-4">
      <Card className="w-full max-w-md" accent="error">
        <CardHeader>
          <Eyebrow>Sign-in failed</Eyebrow>
          <CardTitle className="mt-1">Auth callback returned an error</CardTitle>
          <CardDescription>
            The OAuth round-trip came back with a problem. The full message is
            below; common causes are listed beneath it.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <pre className="bg-paper-100 border border-paper-200 rounded-sm p-3 font-mono text-[12px] text-ink-900 whitespace-pre-wrap break-all">
            {error}
          </pre>
          {diagnostics && (
            <details className="text-[12px] text-ink-700">
              <summary className="cursor-pointer font-mono uppercase tracking-caps text-ink-500">
                Storage diagnostics
              </summary>
              <pre className="mt-2 bg-paper-100 border border-paper-200 rounded-sm p-3 font-mono text-[11px] text-ink-900 whitespace-pre-wrap break-all">
                {JSON.stringify(diagnostics, null, 2)}
              </pre>
            </details>
          )}
          {(isServiceRoleKey || /secret API key/i.test(error)) && (
            <Card accent="error" className="bg-roof-100/40">
              <CardContent className="pt-5 space-y-2 text-[13px] text-roof-700">
                <strong className="block">
                  The configured Supabase key is a{" "}
                  <code className="cl-code">service_role</code> key.
                </strong>
                <p>
                  The browser must use the <strong>anon (publishable)</strong>{" "}
                  key. Open Supabase &rarr; Settings &rarr; API and copy the
                  key labelled <em>anon · public</em> (or{" "}
                  <em>publishable</em>) into your <code>.env</code>:
                </p>
                <pre className="bg-paper-100 border border-paper-200 rounded-sm p-2 font-mono text-[11px] text-ink-900">
                  VITE_SUPABASE_PUBLISHABLE_KEY=eyJ…  # the anon key
                </pre>
              </CardContent>
            </Card>
          )}
          <ul className="space-y-1.5 text-[13px] text-ink-700 list-disc pl-5">
            <li>
              Check Supabase &rarr; Authentication &rarr; URL Configuration:
              <code className="cl-code"> http://localhost:5173 </code>
              must be in <em>Site URL</em> or <em>Additional Redirect URLs</em>.
            </li>
            <li>
              Confirm the key in <code className="cl-code">.env</code> is the
              project&apos;s <strong>anon (public/publishable)</strong> key,
              not the <code className="cl-code">service_role</code> key.
            </li>
            <li>
              In Supabase &rarr; Authentication &rarr; Providers &rarr; GitHub,
              verify the Client ID and Client Secret match the GitHub OAuth app.
            </li>
          </ul>
          <div className="flex gap-3 items-center">
            <button
              type="button"
              onClick={() => navigate("/login", { replace: true })}
              className="font-mono text-[12px] text-lake-700 underline"
            >
              Back to sign in
            </button>
            <button
              type="button"
              onClick={async () => {
                // Drop everything Supabase-related: storage, cookies, in-memory
                // session. After this, the next sign-in starts from scratch.
                try {
                  await supabase.auth.signOut({ scope: "local" });
                } catch {
                  /* ignore — we're going to wipe anyway */
                }
                for (const k of Object.keys(localStorage)) {
                  if (k.startsWith("sb-")) localStorage.removeItem(k);
                }
                for (const k of Object.keys(sessionStorage)) {
                  if (k.startsWith("sb-")) sessionStorage.removeItem(k);
                }
                document.cookie.split(";").forEach((c) => {
                  const eq = c.indexOf("=");
                  const name = (eq > -1 ? c.substring(0, eq) : c).trim();
                  if (name.startsWith("sb-")) {
                    document.cookie = `${name}=;expires=Thu, 01 Jan 1970 00:00:00 GMT;path=/`;
                  }
                });
                navigate("/login", { replace: true });
              }}
              className="font-mono text-[12px] text-roof-700 underline"
            >
              Reset auth state &amp; retry
            </button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
