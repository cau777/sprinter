import { useState } from "react";
import type { FormEvent } from "react";
import { Button, Input } from "@heroui/react";
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
    <main className="relative flex min-h-dvh flex-col items-center justify-center p-6">
      <section className="w-full max-w-[390px] rounded-[18px] border border-[var(--border)] bg-[rgba(17,24,38,.76)] px-9 pt-[34px] pb-[26px] shadow-[0_28px_100px_rgba(0,0,0,.24)] backdrop-blur-lg max-[420px]:px-6">
        <div className="mb-7 flex items-center justify-center gap-2.5 font-display text-[13px] font-bold tracking-[.17em] text-slate-100"><span className="grid size-[29px] place-items-center rounded-[9px] bg-[var(--accent)] text-[var(--bg)] shadow-[0_0_22px_rgba(61,232,255,.2)]"><Sparkles size={18} /></span><span>SPRINTER</span></div>
        <div className="mx-auto mb-3 grid size-[38px] place-items-center rounded-xl border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><LockKeyhole size={19} /></div>
        <p className="mb-2 text-center font-mono text-[11px] tracking-[.13em] text-slate-500">YOUR PRIVATE WORKSPACE</p>
        <h1 className="m-0 text-center font-display text-[21px] font-medium tracking-[-.035em] text-slate-100">Good to have you back.</h1>
        <p className="mt-2 mb-[22px] text-center text-[11px] text-slate-400">Enter your workspace password to continue.</p>
        <form onSubmit={submit} className="grid gap-2">
          <label className="text-[11px] font-semibold text-slate-300" htmlFor="workspace-password">Password</label>
          <Input id="workspace-password" aria-label="Password" className="h-11 w-full rounded-lg border border-[var(--field-border)] bg-[rgba(7,11,19,.5)] px-3 text-sm text-[var(--text-strong)] outline-none focus:border-[var(--accent-line)] focus:ring-2 focus:ring-[var(--accent-soft)]" type="password" autoComplete="current-password" autoFocus required value={password} onChange={(event) => setPassword(event.target.value)} />
          {error && <div className="text-[11px] text-rose-300" role="alert">{error}</div>}
          <Button type="submit" variant="primary" className="mt-1 w-full" isDisabled={submitting || password.length === 0}>
            {submitting ? "Signing in…" : <>Continue <ArrowRight size={15} /></>}
          </Button>
        </form>
        <div className="mt-5 flex items-center justify-center gap-1.5 text-[11px] text-slate-500"><ShieldCheck className="text-emerald-300/70" size={13} /><span>Your conversations stay on this server.</span></div>
      </section>
      <footer className="absolute bottom-[17px] font-mono text-[11px] tracking-[.12em] text-slate-600">SP SPRINT INTO A CLEARER HEADSPACE</footer>
    </main>
  );
}
