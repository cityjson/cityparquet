import { Github } from "lucide-react";
import { useState, type FormEvent } from "react";
import { Navigate, useLocation } from "react-router-dom";

import logoMark from "@/assets/logo-mark.svg";
import { useAuth } from "@/auth/AuthContext";
import { Eyebrow } from "@/components/Eyebrow";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { isServiceRoleKey } from "@/lib/supabase/client";
import { isSupabaseConfigured, supabase } from "@/lib/supabase";

type Status = "idle" | "github" | "email-sending" | "email-sent" | "error";

export default function LoginPage() {
  const { session, loading } = useAuth();
  const location = useLocation();
  const from = (location.state as { from?: string } | null)?.from ?? "/datasets";

  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<Status>("idle");
  const [error, setError] = useState<string | null>(null);

  if (loading) {
    return (
      <div className="flex min-h-screen items-center justify-center bg-paper-50 font-mono text-[12px] text-ink-500">
        Checking session…
      </div>
    );
  }

  if (session) {
    return <Navigate to={from} replace />;
  }

  const requireSupabase = (): boolean => {
    if (!isSupabaseConfigured) {
      setStatus("error");
      setError(
        "Supabase env not set. Configure SUPABASE_URL and the publishable (anon) key in the workspace .env.",
      );
      return false;
    }
    if (isServiceRoleKey) {
      setStatus("error");
      setError(
        "The configured Supabase key is a service_role key. The browser must use the anon (publishable) key — copy that one from Supabase → Settings → API.",
      );
      return false;
    }
    return true;
  };

  async function onGithubSignIn() {
    if (!requireSupabase()) return;
    setStatus("github");
    setError(null);

    // Route the OAuth round-trip through /auth/callback so PKCE exchange
    // errors (redirect-URL mismatch, anon-key wrong, etc.) surface in a
    // friendly UI instead of being swallowed silently. The original
    // destination rides through as `?next=`.
    const redirectTo = `${window.location.origin}/auth/callback?next=${encodeURIComponent(from)}`;
    const { error: signInError } = await supabase.auth.signInWithOAuth({
      provider: "github",
      options: { redirectTo },
    });

    if (signInError) {
      setStatus("error");
      setError(signInError.message);
    }
    // On success, the browser navigates to GitHub — no further work here.
  }

  async function onEmailSubmit(e: FormEvent) {
    e.preventDefault();
    if (!requireSupabase()) return;
    setStatus("email-sending");
    setError(null);

    const { error: signInError } = await supabase.auth.signInWithOtp({ email });
    if (signInError) {
      setStatus("error");
      setError(signInError.message);
      return;
    }
    setStatus("email-sent");
  }

  const githubBusy = status === "github";
  const emailBusy = status === "email-sending";

  return (
    <div className="flex min-h-screen items-center justify-center bg-paper-50 p-4">
      <Card className="w-full max-w-sm">
        <CardHeader>
          <div className="flex items-center gap-3">
            <img src={logoMark} alt="" className="h-8 w-8" aria-hidden="true" />
            <div>
              <Eyebrow>Sign in</Eyebrow>
              <CardTitle className="mt-0.5">CityLake</CardTitle>
            </div>
          </div>
          <CardDescription className="mt-3">
            Sign in with GitHub, or use a magic link to your email.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-5">
          <Button
            type="button"
            className="w-full"
            onClick={onGithubSignIn}
            disabled={githubBusy || emailBusy}
          >
            <Github className="h-4 w-4" />
            {githubBusy ? "Redirecting…" : "Continue with GitHub"}
          </Button>

          <div
            role="separator"
            aria-orientation="horizontal"
            className="relative"
          >
            <div className="border-t border-paper-200" />
            <span className="absolute inset-0 -top-2 mx-auto w-fit bg-white px-2 font-mono text-[10px] uppercase tracking-caps text-ink-500">
              or
            </span>
          </div>

          <form onSubmit={onEmailSubmit} className="space-y-3">
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                type="email"
                required
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                disabled={githubBusy}
              />
            </div>

            <Button
              type="submit"
              variant="secondary"
              className="w-full"
              disabled={emailBusy || githubBusy}
            >
              {emailBusy ? "Sending…" : "Send magic link"}
            </Button>
          </form>

          {status === "email-sent" && (
            <p className="font-mono text-[11px] text-ink-500">
              Check your inbox for a sign-in link.
            </p>
          )}
          {status === "error" && error && (
            <p className="font-mono text-[12px] text-roof-700">{error}</p>
          )}
          {!isSupabaseConfigured && status === "idle" && (
            <p className="font-mono text-[11px] text-ink-500">
              Heads up: Supabase env is not configured yet — sign-in will
              fail until <code className="cl-code">.env</code> is filled in.
            </p>
          )}
        </CardContent>
      </Card>
    </div>
  );
}
