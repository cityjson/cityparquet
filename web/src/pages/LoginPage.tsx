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
import { supabase } from "@/lib/supabase";

export default function LoginPage() {
  const { session, loading } = useAuth();
  const location = useLocation();
  const from = (location.state as { from?: string } | null)?.from ?? "/datasets";

  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<"idle" | "sending" | "sent" | "error">(
    "idle",
  );
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

  const supabaseConfigured =
    !!import.meta.env.VITE_SUPABASE_URL &&
    !!import.meta.env.VITE_SUPABASE_ANON_KEY;

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    if (!supabaseConfigured) {
      setStatus("error");
      setError(
        "Supabase env not set. Configure VITE_SUPABASE_URL and VITE_SUPABASE_ANON_KEY in .env.local.",
      );
      return;
    }
    setStatus("sending");
    setError(null);

    const { error: signInError } = await supabase.auth.signInWithOtp({ email });
    if (signInError) {
      setStatus("error");
      setError(signInError.message);
      return;
    }
    setStatus("sent");
  }

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
            Enter your email and we&apos;ll send you a magic link.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <form onSubmit={onSubmit} className="space-y-4">
            <div className="space-y-2">
              <Label htmlFor="email">Email</Label>
              <Input
                id="email"
                type="email"
                required
                autoFocus
                value={email}
                onChange={(e) => setEmail(e.target.value)}
              />
            </div>

            <Button
              type="submit"
              className="w-full"
              disabled={status === "sending"}
            >
              {status === "sending" ? "Sending…" : "Send magic link"}
            </Button>

            {status === "sent" && (
              <p className="font-mono text-[11px] text-ink-500">
                Check your inbox for a sign-in link.
              </p>
            )}
            {status === "error" && error && (
              <p className="font-mono text-[12px] text-roof-700">{error}</p>
            )}
            {!supabaseConfigured && status === "idle" && (
              <p className="font-mono text-[11px] text-ink-500">
                Heads up: Supabase env is not configured yet — sign-in will
                fail until <code className="cl-code">.env.local</code> is filled
                in.
              </p>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  );
}
