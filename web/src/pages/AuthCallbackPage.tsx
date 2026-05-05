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
export default function AuthCallbackPage() {
  const navigate = useNavigate();
  const [searchParams] = useSearchParams();
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const errorParam =
      searchParams.get("error_description") || searchParams.get("error");
    if (errorParam) {
      setError(decodeURIComponent(errorParam));
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
          setError(`Code exchange failed: ${exchangeError.message}`);
          return;
        }
      } else {
        // Implicit flow: token sat in the URL hash. supabase-js handles that
        // on import; nothing to do here besides confirm.
        const { error: sessionError } = await supabase.auth.getSession();
        if (cancelled) return;
        if (sessionError) {
          setError(sessionError.message);
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
          <ul className="space-y-1.5 text-[13px] text-ink-700 list-disc pl-5">
            <li>
              Check Supabase &rarr; Authentication &rarr; URL Configuration:
              <code className="cl-code"> http://localhost:5173 </code>
              must be in <em>Site URL</em> or <em>Additional Redirect URLs</em>.
            </li>
            <li>
              Confirm the <code className="cl-code">SUPABASE_KEY</code> in
              <code className="cl-code"> .env</code> is the project&apos;s{" "}
              <strong>anon (public)</strong> key, not the service-role key.
            </li>
            <li>
              In Supabase &rarr; Authentication &rarr; Providers &rarr; GitHub,
              verify the Client ID and Client Secret match the GitHub OAuth app.
            </li>
          </ul>
          <button
            type="button"
            onClick={() => navigate("/login", { replace: true })}
            className="font-mono text-[12px] text-lake-700 underline"
          >
            Back to sign in
          </button>
        </CardContent>
      </Card>
    </div>
  );
}
