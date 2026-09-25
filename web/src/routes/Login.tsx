import { useState } from "react";
import type { FormEvent } from "react";
import { Button } from "@heroui/react";
import { ArrowRight, LockKeyhole, ShieldCheck, Sparkles } from "lucide-react";
import { useSearch } from "@tanstack/react-router";
import { ApiError, apiRequest, jsonBody } from "../api/client";

export function Login() {
  const { redirect: requestedPath } = useSearch({ from: "/login" });
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string>();
  const [submitting, setSubmitting] = useState(false);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError(undefined);
    setSubmitting(true);
    try {
      await apiRequest<{ ok: true }>("/api/auth/login", { method: "POST", body: jsonBody({ password }) }, { redirectOnUnauthorized: false });
      const destination = requestedPath?.startsWith("/") && !requestedPath.startsWith("//") ? requestedPath : "/";
      window.location.assign(destination);
    } catch (cause) {
      setError(cause instanceof ApiError && cause.status === 429 ? "Too many attempts. Please wait a moment and try again." : cause instanceof Error ? cause.message : "Unable to sign in. Try again.");
      setSubmitting(false);
    }
  }

  return (
    <main className="login-frame">
      <section className="login-card">
        <div className="login-brand"><span className="brand-mark"><Sparkles size={18} /></span><span>SPRINTER</span></div>
        <div className="login-lock"><LockKeyhole size={19} /></div>
        <p className="login-eyebrow">YOUR PRIVATE WORKSPACE</p>
        <h1>Good to have you back.</h1>
        <p className="login-description">Enter your workspace password to continue.</p>
        <form onSubmit={submit} className="login-form">
          <label htmlFor="workspace-password">Password</label>
          <input id="workspace-password" aria-label="Password" type="password" autoComplete="current-password" autoFocus required value={password} onChange={(event) => setPassword(event.target.value)} />
          {error && <div className="login-error" role="alert">{error}</div>}
          <Button type="submit" className="login-submit" isDisabled={submitting || password.length === 0}>
            {submitting ? "Signing in…" : <>Continue <ArrowRight size={15} /></>}
          </Button>
        </form>
        <div className="login-privacy"><ShieldCheck size={13} /><span>Your conversations stay on this server.</span></div>
      </section>
      <footer className="login-footer">SP SPRINT INTO A CLEARER HEADSPACE</footer>
    </main>
  );
}
