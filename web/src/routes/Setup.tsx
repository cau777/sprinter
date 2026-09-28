import { useState } from "react";
import { Button } from "@heroui/react";
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
    <section className="setup-stage">
      <div className="setup-card">
        <div className="setup-progress"><span className={step === 1 ? "is-current" : "is-complete"}>01</span><i /><span className={step === 2 ? "is-current" : ""}>02</span></div>
        <div className="setup-brand"><span className="brand-mark"><Sparkles size={16} /></span><span>SPRINTER SETUP</span></div>
        {!online && <p className="settings-offline-note" role="status">You’re offline. Reconnect to finish setup.</p>}
        <fieldset disabled={!online} className="setup-readonly-fieldset">
        {step === 1 ? <>
          <div className="setup-icon"><KeyRound size={20} /></div>
          <p className="login-eyebrow">CONNECT YOUR MODEL PROVIDER</p>
          <h1>Bring your own key.</h1>
          <p className="setup-copy">Sprinter uses your OpenRouter key to connect to the models you choose. It is encrypted on this server.</p>
          {existingKey?.set && <div className="setup-key-status"><ShieldCheck size={14} /> Key {existingKey.hint ? `ending in ${existingKey.hint}` : "is already connected"}</div>}
          <form className="setup-form" onSubmit={(event) => { event.preventDefault(); setError(undefined); if (!apiKey.trim() && existingKey?.set) setStep(2); else saveKey.mutate(); }}>
            <label htmlFor="openrouter-key">OpenRouter API key</label>
            <input id="openrouter-key" type="password" autoComplete="off" placeholder="sk-or-v1-…" value={apiKey} onChange={(event) => setApiKey(event.target.value)} required={!existingKey?.set} />
            {error && <div className="setup-error" role="alert">{error}</div>}
            <Button type="submit" className="setup-primary" isDisabled={saveKey.isPending || (!apiKey.trim() && !existingKey?.set)}>{saveKey.isPending ? "Checking key…" : <>{existingKey?.set && !apiKey.trim() ? "Continue" : "Validate and continue"} <ArrowRight size={15} /></>}</Button>
          </form>
          <p className="setup-footnote">Your key stays private and is never sent to Sprinter’s authors.</p>
        </> : <>
          <div className="setup-icon"><Sparkles size={20} /></div>
          <p className="login-eyebrow">MAKE THIS SPACE YOURS</p>
          <h1>Choose your default model.</h1>
          <p className="setup-copy">Pick the model Sprinter will use for new conversations. You can change it anytime.</p>
          {modelsQuery.isLoading ? <div className="setup-loading">Loading available models…</div> : modelsQuery.isError ? <div className="setup-error" role="alert">{modelsQuery.error.message}</div> : <ModelPicker models={modelsQuery.data?.items ?? []} value={model || settingsQuery.data?.default_model || null} onChange={setModel} isDisabled={!online} placeholder="Search available models…" />}
          {error && <div className="setup-error" role="alert">{error}</div>}
          <div className="setup-footer-actions"><Button className="setup-back" variant="ghost" onPress={() => { setError(undefined); setStep(1); }}><ArrowLeft size={14} /> Back</Button><Button className="setup-primary" isDisabled={!model && !settingsQuery.data?.default_model || saveModel.isPending} onPress={() => { setError(undefined); saveModel.mutate(); }}>{saveModel.isPending ? "Saving…" : <>Finish setup <ArrowRight size={15} /></>}</Button></div>
        </>}
        </fieldset>
      </div>
      <p className="setup-note">Your workspace. Your key. Your conversation.</p>
    </section>
  );
}
