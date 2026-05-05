import { useState, type FormEvent } from "react";

import { supabase } from "@/lib/supabase";

export default function LoginPage() {
  const [email, setEmail] = useState("");
  const [status, setStatus] = useState<"idle" | "sending" | "sent" | "error">(
    "idle",
  );
  const [error, setError] = useState<string | null>(null);

  async function onSubmit(e: FormEvent) {
    e.preventDefault();
    setStatus("sending");
    setError(null);

    const { error } = await supabase.auth.signInWithOtp({ email });
    if (error) {
      setStatus("error");
      setError(error.message);
      return;
    }
    setStatus("sent");
  }

  return (
    <div className="min-h-screen flex items-center justify-center p-8">
      <form
        onSubmit={onSubmit}
        className="w-full max-w-sm space-y-4 rounded-lg border border-border bg-background p-6 shadow-sm"
      >
        <h1 className="text-2xl font-semibold">Sign in to CityLake</h1>
        <p className="text-sm text-muted-foreground">
          Enter your email and we'll send you a magic link.
        </p>

        <label className="block">
          <span className="text-sm font-medium">Email</span>
          <input
            type="email"
            required
            autoFocus
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            className="mt-1 block w-full rounded-md border border-border bg-background px-3 py-2"
          />
        </label>

        <button
          type="submit"
          disabled={status === "sending"}
          className="w-full rounded-md bg-primary px-4 py-2 text-primary-foreground disabled:opacity-50"
        >
          {status === "sending" ? "Sending…" : "Send magic link"}
        </button>

        {status === "sent" && (
          <p className="text-sm text-muted-foreground">
            Check your inbox for a sign-in link.
          </p>
        )}
        {status === "error" && error && (
          <p className="text-sm text-destructive">{error}</p>
        )}
      </form>
    </div>
  );
}
