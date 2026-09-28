import { useState } from "react";
import { Button, Input } from "@heroui/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, ArrowRight, KeyRound, ShieldCheck, Sparkles } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { ApiError } from "../api/client";
import { fetchModels, fetchSettings, updateSettings } from "../api/settings";
import { ModelPicker } from "../components/ModelPicker";
import { useOnlineStatus } from "../api/useOnlineStatus";

export function Setup() {
  const online = useOnlineStatus();
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: fetchSettings });
  const [step, setStep] = useState<1 | 2>(1);
  const [apiKey, setApiKey] = useState("");
  const [model, setModel] = useState("");
  const [error, setError] = useState<string>();
  const modelsQuery = useQuery({ queryKey: ["models"], queryFn: fetchModels, enabled: settingsQuery.data?.openrouter_api_key.set === true });

  const saveKey = useMutation({
    mutationFn: () => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to finish setup.");
      return updateSettings({ openrouter_api_key: apiKey.trim() });
    },
    onSuccess: (settings) => {
      queryClient.setQueryData(["settings"], settings);
      void queryClient.invalidateQueries({ queryKey: ["models"] });
      setError(undefined);
      setStep(2);
    },
    onError: (cause) => setError(cause instanceof ApiError ? cause.message : "Could not validate this key. Check the key and try again."),
  });

  const saveModel = useMutation({
    mutationFn: () => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to finish setup.");
      return updateSettings({ default_model: model || settingsQuery.data?.default_model });
    },
    onSuccess: (settings) => {
      queryClient.setQueryData(["settings"], settings);
      void navigate({ to: "/" });
    },
    onError: (cause) => setError(cause instanceof Error ? cause.message : "Could not save the model."),
  });

  const existingKey = settingsQuery.data?.openrouter_api_key;

  return (
    <section className="grid min-h-0 flex-1 place-items-center overflow-y-auto overscroll-contain px-[18px] py-7 max-[720px]:px-3 max-[720px]:pb-[calc(20px+env(safe-area-inset-bottom))]">
      <div className="w-full max-w-[465px] rounded-[17px] border border-[var(--border)] bg-[rgba(17,24,38,.72)] px-[34px] pt-[30px] pb-[25px] shadow-[0_24px_90px_rgba(0,0,0,.21)] backdrop-blur-md max-[720px]:px-5 max-[720px]:py-[25px]">
        <div className="mb-6 -mt-1 flex items-center justify-center gap-2 font-mono text-[9px] text-slate-600"><span className={`grid size-[22px] place-items-center rounded-full border ${step === 1 ? "border-[var(--accent)] bg-[var(--accent)] text-[var(--on-accent)]" : "border-[var(--accent-line)] text-[var(--accent)]"}`}>01</span><i className="h-px w-[31px] bg-[var(--border)]" /><span className={`grid size-[22px] place-items-center rounded-full border ${step === 2 ? "border-[var(--accent)] bg-[var(--accent)] text-[var(--on-accent)]" : "border-[var(--border)] text-slate-600"}`}>02</span></div>
        <div className="mb-6 flex items-center justify-center gap-2 font-display text-[11px] font-bold tracking-[.15em] text-slate-200"><span className="grid size-6 place-items-center rounded-lg bg-[var(--accent)] text-[var(--bg)] shadow-[0_0_22px_rgba(61,232,255,.2)]"><Sparkles size={16} /></span><span>SPRINTER SETUP</span></div>
        {!online && <p className="settings-offline-note" role="status">You’re offline. Reconnect to finish setup.</p>}
        <fieldset disabled={!online} className="m-0 min-w-0 border-0 p-0">
        {step === 1 ? <>
          <div className="mx-auto mb-3.5 grid size-[42px] place-items-center rounded-[13px] border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><KeyRound size={20} /></div>
          <p className="mb-2 text-center font-mono text-[9px] tracking-[.13em] text-slate-500">CONNECT YOUR MODEL PROVIDER</p>
          <h1 className="m-0 text-center font-display text-xl font-medium tracking-[-.035em] text-slate-100">Bring your own key.</h1>
          <p className="mx-auto mt-2 mb-5 max-w-[345px] text-center text-[10px] leading-[1.7] text-slate-400">Sprinter uses your OpenRouter key to connect to the models you choose. It is encrypted on this server.</p>
          {existingKey?.set && <div className="-mt-2 mb-4 flex items-center justify-center gap-1.5 text-[10px] text-emerald-300"><ShieldCheck size={14} /> Key {existingKey.hint ? `ending in ${existingKey.hint}` : "is already connected"}</div>}
          <form className="grid gap-2" onSubmit={(event) => { event.preventDefault(); setError(undefined); if (!apiKey.trim() && existingKey?.set) setStep(2); else saveKey.mutate(); }}>
            <label className="text-[10px] font-semibold text-slate-300" htmlFor="openrouter-key">OpenRouter API key</label>
            <Input id="openrouter-key" className="h-10 w-full rounded-lg border border-[var(--field-border)] bg-[rgba(7,11,19,.5)] px-3 font-mono text-xs text-[var(--text-strong)] outline-none placeholder:text-slate-500 focus:border-[var(--accent-line)] focus:ring-2 focus:ring-[var(--accent-soft)]" type="password" autoComplete="off" placeholder="sk-or-v1-…" value={apiKey} onChange={(event) => setApiKey(event.target.value)} required={!existingKey?.set} />
            {error && <div className="text-[10px] text-rose-300" role="alert">{error}</div>}
            <Button type="submit" variant="primary" className="mt-1 min-w-40" isDisabled={saveKey.isPending || (!apiKey.trim() && !existingKey?.set)}>{saveKey.isPending ? "Checking key…" : <>{existingKey?.set && !apiKey.trim() ? "Continue" : "Validate and continue"} <ArrowRight size={15} /></>}</Button>
          </form>
          <p className="mt-4 text-center text-[9px] text-slate-500">Your key stays private and is never sent to Sprinter’s authors.</p>
        </> : <>
          <div className="mx-auto mb-3.5 grid size-[42px] place-items-center rounded-[13px] border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><Sparkles size={20} /></div>
          <p className="mb-2 text-center font-mono text-[9px] tracking-[.13em] text-slate-500">MAKE THIS SPACE YOURS</p>
          <h1 className="m-0 text-center font-display text-xl font-medium tracking-[-.035em] text-slate-100">Choose your default model.</h1>
          <p className="mx-auto mt-2 mb-5 max-w-[345px] text-center text-[10px] leading-[1.7] text-slate-400">Pick the model Sprinter will use for new conversations. You can change it anytime.</p>
          {modelsQuery.isLoading ? <div className="py-4 text-center text-[10px] text-slate-400">Loading available models…</div> : modelsQuery.isError ? <div className="text-[10px] text-rose-300" role="alert">{modelsQuery.error.message}</div> : <ModelPicker models={modelsQuery.data?.items ?? []} value={model || settingsQuery.data?.default_model || null} onChange={setModel} isDisabled={!online} placeholder="Search available models…" />}
          {error && <div className="text-[10px] text-rose-300" role="alert">{error}</div>}
          <div className="mt-[18px] flex items-center justify-between"><Button className="flex h-10 items-center gap-1.5 text-[10px] text-slate-400" variant="ghost" onPress={() => { setError(undefined); setStep(1); }}><ArrowLeft size={14} /> Back</Button><Button variant="primary" className="mt-1 min-w-40" isDisabled={!model && !settingsQuery.data?.default_model || saveModel.isPending} onPress={() => { setError(undefined); saveModel.mutate(); }}>{saveModel.isPending ? "Saving…" : <>Finish setup <ArrowRight size={15} /></>}</Button></div>
        </>}
        </fieldset>
      </div>
      <p className="-mb-1 mt-4 self-end text-center text-[9px] text-slate-500 max-[720px]:hidden">Your workspace. Your key. Your conversation.</p>
    </section>
  );
}
