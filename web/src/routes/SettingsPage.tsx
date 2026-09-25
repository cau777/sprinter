import { useEffect, useState } from "react";
import { Button } from "@heroui/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Check, KeyRound, Save, Settings2, ShieldCheck, Star } from "lucide-react";
import { ApiError } from "../api/client";
import { fetchModels, fetchSettings, updateSettings } from "../api/settings";
import { ModelPicker } from "../components/ModelPicker";

export function SettingsPage() {
  const queryClient = useQueryClient();
  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: fetchSettings });
  const modelQuery = useQuery({ queryKey: ["models"], queryFn: fetchModels, enabled: settingsQuery.data?.openrouter_api_key.set === true });
  const [apiKey, setApiKey] = useState("");
  const [instructions, setInstructions] = useState("");
  const [saved, setSaved] = useState<string>();
  const [error, setError] = useState<string>();

  useEffect(() => {
    if (settingsQuery.data) setInstructions(settingsQuery.data.custom_instructions);
  }, [settingsQuery.data]);

  const save = useMutation({
    mutationFn: (patch: Parameters<typeof updateSettings>[0]) => updateSettings(patch),
    onSuccess: (settings, patch) => {
      queryClient.setQueryData(["settings"], settings);
      setError(undefined);
      setSaved(patch.openrouter_api_key !== undefined ? "API key updated" : patch.custom_instructions !== undefined ? "Instructions saved" : "Settings saved");
      setApiKey("");
    },
    onError: (cause) => setError(cause instanceof ApiError ? cause.message : "Could not save settings."),
  });

  const settings = settingsQuery.data;
  const models = modelQuery.data?.items ?? [];
  const favoriteModels = settings?.favorite_models ?? [];

  function changeFavorites(next: string[]) {
    setSaved(undefined);
    setError(undefined);
    save.mutate({ favorite_models: next });
  }

  function moveFavorite(index: number, direction: -1 | 1) {
    const to = index + direction;
    if (to < 0 || to >= favoriteModels.length) return;
    const next = [...favoriteModels];
    [next[index], next[to]] = [next[to]!, next[index]!];
    changeFavorites(next);
  }

  return (
    <section className="settings-page">
      <header className="settings-heading"><div className="settings-heading-icon"><Settings2 size={18} /></div><div><p className="eyebrow">YOUR WORKSPACE</p><h1>Settings</h1><p>Keep your workspace tuned to the way you think.</p></div></header>
      {settingsQuery.isLoading ? <div className="settings-loading">Loading settings…</div> : settingsQuery.isError ? <div className="settings-error" role="alert">{settingsQuery.error.message}</div> : <>
        {error && <div className="settings-toast-error" role="alert">{error}</div>}
        {saved && <div className="settings-toast-success" role="status"><Check size={14} />{saved}</div>}

        <section className="settings-section" aria-labelledby="provider-heading">
          <div className="settings-section-heading"><div className="settings-section-icon"><KeyRound size={16} /></div><div><h2 id="provider-heading">OpenRouter</h2><p>Your key connects Sprinter to model providers.</p></div></div>
          <div className="settings-card">
            <div className="settings-key-status"><span className={settings?.openrouter_api_key.set ? "status-led is-good" : "status-led"} /><span>{settings?.openrouter_api_key.set ? settings.openrouter_api_key.readable ? "Key connected" : "Key needs to be entered again" : "No key saved yet"}</span>{settings?.openrouter_api_key.hint && <code>{settings.openrouter_api_key.hint}</code>}</div>
            <form className="settings-key-form" onSubmit={(event) => { event.preventDefault(); setSaved(undefined); setError(undefined); save.mutate({ openrouter_api_key: apiKey.trim() }); }}>
              <label htmlFor="settings-openrouter-key">{settings?.openrouter_api_key.set ? "Replace API key" : "API key"}</label>
              <div className="settings-key-row"><input id="settings-openrouter-key" type="password" autoComplete="off" placeholder="sk-or-v1-…" value={apiKey} onChange={(event) => setApiKey(event.target.value)} /><Button type="submit" className="settings-save-button" isDisabled={!apiKey.trim() || save.isPending}>{save.isPending ? "Validating…" : "Validate and save"}</Button></div>
            </form>
            <p className="settings-private-note"><ShieldCheck size={13} /> Encrypted at rest. The full key is never shown after saving.</p>
            {settings?.openrouter_api_key.set && <button className="settings-danger-link" type="button" onClick={() => { setSaved(undefined); setError(undefined); save.mutate({ openrouter_api_key: null }); }}>Remove saved key</button>}
          </div>
        </section>

        <section className="settings-section" aria-labelledby="models-heading">
          <div className="settings-section-heading"><div className="settings-section-icon"><Star size={16} /></div><div><h2 id="models-heading">Models</h2><p>Choose defaults and keep favorite models close.</p></div></div>
          {!settings?.openrouter_api_key.set ? <div className="settings-card settings-empty-note">Add an OpenRouter key to browse and set models.</div> : modelQuery.isLoading ? <div className="settings-card settings-empty-note">Loading available models…</div> : modelQuery.isError ? <div className="settings-card settings-error" role="alert">{modelQuery.error.message}</div> : <div className="settings-card settings-models-card">
            <div className="settings-model-fields">
              <label><span>Default model</span><ModelPicker models={models} value={settings.default_model} favorites={favoriteModels} onChange={(default_model) => { setSaved(undefined); save.mutate({ default_model }); }} onToggleFavorite={(id) => changeFavorites(favoriteModels.includes(id) ? favoriteModels.filter((item) => item !== id) : [...favoriteModels, id])} /></label>
              <label><span>Title model <small>Optional. Uses the conversation model when empty.</small></span><div className="settings-title-model"><ModelPicker models={models} value={settings.title_model} favorites={favoriteModels} onChange={(title_model) => { setSaved(undefined); save.mutate({ title_model }); }} placeholder="Use conversation model" /><button type="button" onClick={() => { setSaved(undefined); save.mutate({ title_model: null }); }}>Clear</button></div></label>
            </div>
            <div className="favorite-list-heading">FAVORITES <span>Shown first in the model picker</span></div>
            {favoriteModels.length === 0 ? <div className="favorite-empty">Star a model in a picker to add it here.</div> : <ul className="favorite-list">{favoriteModels.map((id, index) => {
              const item = models.find((model) => model.id === id);
              return <li key={id}><Star size={13} fill="currentColor" /><span>{item?.name ?? id}<small>{id}</small></span><button type="button" aria-label={`Move ${id} up`} disabled={index === 0} onClick={() => moveFavorite(index, -1)}><ArrowUp size={13} /></button><button type="button" aria-label={`Move ${id} down`} disabled={index === favoriteModels.length - 1} onClick={() => moveFavorite(index, 1)}><ArrowDown size={13} /></button><button className="favorite-remove" type="button" onClick={() => changeFavorites(favoriteModels.filter((fav) => fav !== id))}>Remove</button></li>;
            })}</ul>}
          </div>}
        </section>

        <section className="settings-section" aria-labelledby="instructions-heading">
          <div className="settings-section-heading"><div className="settings-section-icon"><Save size={16} /></div><div><h2 id="instructions-heading">Custom instructions</h2><p>Context Sprinter will include in every conversation.</p></div></div>
          <div className="settings-card instructions-card">
            <label htmlFor="custom-instructions">Instructions</label>
            <textarea id="custom-instructions" value={instructions} onChange={(event) => setInstructions(event.target.value)} placeholder="For example: be concise, ask before making assumptions, and use metric units." rows={5} />
            <div className="instructions-footer"><span>Sent as a private system instruction with each request.</span><Button className="settings-save-button" isDisabled={instructions === settings?.custom_instructions || save.isPending} onPress={() => { setSaved(undefined); setError(undefined); save.mutate({ custom_instructions: instructions }); }}>{save.isPending ? "Saving…" : "Save instructions"}</Button></div>
          </div>
        </section>
      </>}
    </section>
  );
}
