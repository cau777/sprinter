import { useEffect, useState } from "react";
import { Button } from "@heroui/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Check, CreditCard, FileText, KeyRound, LogOut, Save, Settings2, ShieldCheck, Star } from "lucide-react";
import { ApiError } from "../api/client";
import { fetchModels, fetchSessions, fetchSettings, fetchUsage, revokeAllSessions, revokeSession, updateSettings } from "../api/settings";
import { ModelPicker } from "../components/ModelPicker";
import { useOnlineStatus } from "../api/useOnlineStatus";
import { clearPersistedQueryCache } from "../api/queryPersistence";

export function SettingsPage() {
  const online = useOnlineStatus();
  const queryClient = useQueryClient();
  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: fetchSettings });
  const modelQuery = useQuery({ queryKey: ["models"], queryFn: fetchModels, enabled: settingsQuery.data?.openrouter_api_key.set === true });
  const usageQuery = useQuery({ queryKey: ["usage"], queryFn: fetchUsage, staleTime: 60_000 });
  const sessionsQuery = useQuery({ queryKey: ["sessions"], queryFn: fetchSessions });
  const [apiKey, setApiKey] = useState("");
  const [instructions, setInstructions] = useState("");
  const [uploadDraft, setUploadDraft] = useState<{ image_bytes: number; pdf_bytes: number; text_bytes: number; files_per_message: number; total_prompt_bytes: number }>();
  const [saved, setSaved] = useState<string>();
  const [error, setError] = useState<string>();

  useEffect(() => {
    if (settingsQuery.data) setInstructions(settingsQuery.data.custom_instructions);
  }, [settingsQuery.data]);

  const save = useMutation({
    mutationFn: (patch: Parameters<typeof updateSettings>[0]) => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to change settings.");
      return updateSettings(patch);
    },
    onSuccess: (settings, patch) => {
      queryClient.setQueryData(["settings"], settings);
      setError(undefined);
      setSaved(patch.openrouter_api_key !== undefined ? "API key updated" : patch.custom_instructions !== undefined ? "Instructions saved" : "Settings saved");
      setApiKey("");
    },
    onError: (cause) => setError(cause instanceof ApiError ? cause.message : "Could not save settings."),
  });
  const logoutAfterRevocation = () => {
    const clearingCache = clearPersistedQueryCache().catch(() => undefined);
    queryClient.clear();
    void clearingCache.finally(() => window.location.assign("/login"));
  };
  const revokeOne = useMutation({ mutationFn: (id: string) => {
    if (!navigator.onLine) throw new Error("You’re offline. Reconnect to revoke sessions.");
    return revokeSession(id);
  }, onSuccess: (_, id) => {
    if (sessionsQuery.data?.some((session) => session.id === id && session.current)) logoutAfterRevocation();
    else void queryClient.invalidateQueries({ queryKey: ["sessions"] });
  }, onError: (cause) => setError(cause instanceof ApiError ? cause.message : "Could not revoke session.") });
  const revokeEvery = useMutation({ mutationFn: () => {
    if (!navigator.onLine) throw new Error("You’re offline. Reconnect to revoke sessions.");
    return revokeAllSessions();
  }, onSuccess: logoutAfterRevocation, onError: (cause) => setError(cause instanceof ApiError ? cause.message : "Could not revoke sessions.") });

  const settings = settingsQuery.data;
  const models = modelQuery.data?.items ?? [];
  const favoriteModels = settings?.favorite_models ?? [];
  const uploadLimits = uploadDraft ?? settings?.upload_limits;

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
        {!online && <div className="settings-offline-note" role="status">You’re offline. Settings are read-only until you reconnect.</div>}
        {error && <div className="settings-toast-error" role="alert">{error}</div>}
        {saved && <div className="settings-toast-success" role="status"><Check size={14} />{saved}</div>}

        <fieldset disabled={!online} className="settings-readonly-fieldset">
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

        <section className="settings-section" aria-labelledby="files-heading">
          <div className="settings-section-heading"><div className="settings-section-icon"><FileText size={16} /></div><div><h2 id="files-heading">Files</h2><p>Choose how PDFs are parsed and set upload limits.</p></div></div>
          <div className="settings-card settings-files-card">
            <label className="settings-select-field"><span>PDF parsing engine</span><select value={settings?.pdf_engine ?? "cloudflare-ai"} disabled={save.isPending} onChange={(event) => { setSaved(undefined); setError(undefined); save.mutate({ pdf_engine: event.target.value }); }}><option value="cloudflare-ai">Cloudflare AI · free</option><option value="mistral-ocr">Mistral OCR</option><option value="native">Native</option></select><small>Applies to PDFs attached to new messages.</small></label>
            {uploadLimits && <>
              <div className="settings-limit-grid">
                <LimitInput label="Images · MB per file" value={uploadLimits.image_bytes} scale={1024 * 1024} max={40} onChange={(n) => setUploadDraft({ ...uploadLimits, image_bytes: n })} />
                <LimitInput label="PDFs · MB per file" value={uploadLimits.pdf_bytes} scale={1024 * 1024} max={100} onChange={(n) => setUploadDraft({ ...uploadLimits, pdf_bytes: n })} />
                <LimitInput label="Text · MB per file" value={uploadLimits.text_bytes} scale={1024 * 1024} max={5} onChange={(n) => setUploadDraft({ ...uploadLimits, text_bytes: n })} />
                <LimitInput label="Files per message" value={uploadLimits.files_per_message} scale={1} max={20} onChange={(n) => setUploadDraft({ ...uploadLimits, files_per_message: n })} />
                <LimitInput label="Total prompt · MB" value={uploadLimits.total_prompt_bytes} scale={1024 * 1024} max={200} onChange={(n) => setUploadDraft({ ...uploadLimits, total_prompt_bytes: n })} />
              </div>
              <div className="settings-action-footer"><small>Server ceilings: images 40 MB, PDFs 100 MB, text 5 MB, 20 files, total 200 MB.</small><Button className="settings-save-button" isDisabled={!uploadDraft || save.isPending} onPress={() => { setSaved(undefined); setError(undefined); save.mutate({ upload_limits: uploadDraft }, { onSuccess: () => setUploadDraft(undefined) }); }}>{save.isPending ? "Saving…" : "Save upload limits"}</Button></div>
            </>}
          </div>
        </section>

        <section className="settings-section" aria-labelledby="usage-heading">
          <div className="settings-section-heading"><div className="settings-section-icon"><CreditCard size={16} /></div><div><h2 id="usage-heading">Usage</h2><p>Provider balance and recorded model spend.</p></div></div>
          {usageQuery.isLoading ? <div className="settings-card settings-empty-note">Loading usage…</div> : usageQuery.isError ? <div className="settings-card settings-error" role="alert">{usageQuery.error.message}</div> : usageQuery.data && <div className="settings-card settings-usage-card">
            <div className="usage-topline"><div><span>OPENROUTER BALANCE</span><strong>{!settings?.openrouter_api_key.set ? "Set an API key to see your balance" : usageQuery.data.balance == null ? "Unavailable" : money(usageQuery.data.balance)}</strong></div><div className="usage-periods">{([["Today", usageQuery.data.totals.today], ["7 days", usageQuery.data.totals.d7], ["30 days", usageQuery.data.totals.d30], ["All time", usageQuery.data.totals.all]] as const).map(([label, total]) => <div key={label}><span>{label}</span><strong>{money(total.cost)}</strong><small>{tokens(total.prompt_tokens + total.completion_tokens)} tokens</small></div>)}</div></div>
            <div className="usage-daily"><div><strong>Daily spend</strong><span>Last 30 days · local time</span></div><Sparkline values={usageQuery.data.daily.map((item) => item.cost)} /><div className="usage-chart-labels"><span>{usageQuery.data.daily[0]?.day ?? ""}</span><span>{usageQuery.data.daily.at(-1)?.day ?? ""}</span></div></div>
            <div className="usage-breakdowns"><div><h3>By model</h3>{usageQuery.data.by_model.length ? <ul>{usageQuery.data.by_model.map((row) => <li key={row.model}><span>{row.model}<small>{tokens(row.prompt_tokens + row.completion_tokens)} tokens</small></span><strong>{money(row.cost)}</strong></li>)}</ul> : <p>No recorded model usage yet.</p>}</div><div><h3>By chat <small>Top 20</small></h3>{usageQuery.data.by_chat.length ? <ul>{usageQuery.data.by_chat.map((row) => <li key={row.chat_id}><span>{row.chat_title || "Untitled chat"}{row.title_cost > 0 && <small>Includes {money(row.title_cost)} title generation</small>}</span><strong>{money(row.cost)}</strong></li>)}</ul> : <p>No recorded chat usage yet.</p>}</div></div>
            <p className="usage-footnote">Spend totals use recorded message costs. Messages without provider cost data may be omitted.</p>
          </div>}
        </section>

        <section className="settings-section" aria-labelledby="sessions-heading">
          <div className="settings-section-heading"><div className="settings-section-icon"><LogOut size={16} /></div><div><h2 id="sessions-heading">Active sessions</h2><p>Devices currently signed in to your workspace.</p></div></div>
          {sessionsQuery.isLoading ? <div className="settings-card settings-empty-note">Loading sessions…</div> : sessionsQuery.isError ? <div className="settings-card settings-error" role="alert">{sessionsQuery.error.message}</div> : <div className="settings-card settings-sessions-card">
            <div className="sessions-toolbar"><span>{sessionsQuery.data?.length ?? 0} active {(sessionsQuery.data?.length ?? 0) === 1 ? "session" : "sessions"}</span><button type="button" disabled={!sessionsQuery.data?.length || revokeEvery.isPending} onClick={() => revokeEvery.mutate()}>{revokeEvery.isPending ? "Logging out…" : "Log out all sessions"}</button></div>
            {!sessionsQuery.data?.length ? <p className="settings-empty-note">No active sessions.</p> : <ul className="settings-session-list">{sessionsQuery.data.map((session) => <li key={session.id}><span className="session-device">{session.user_agent || "Unknown device"}{session.current && <b>THIS DEVICE</b>}<small>{session.ip || "Unknown address"} · Signed in {dateTime(session.created_at)} · Last active {dateTime(session.last_seen_at)}</small></span><button type="button" disabled={revokeOne.isPending} onClick={() => revokeOne.mutate(session.id)}>{session.current ? "Log out" : "Revoke"}</button></li>)}</ul>}
          </div>}
        </section>
        </fieldset>
      </>}
    </section>
  );
}

function LimitInput({ label, value, scale, max, onChange }: { label: string; value: number; scale: number; max: number; onChange: (value: number) => void }) {
  const shown = value / scale;
  return <label className="settings-limit-input"><span>{label}</span><input type="number" min={scale === 1 ? 1 : 0.01} max={max} step={scale === 1 ? 1 : 0.1} value={Number.isInteger(shown) ? shown : Number(shown.toFixed(2))} onChange={(event) => { const parsed = Number(event.target.value); if (Number.isFinite(parsed)) onChange(Math.round(parsed * scale)); }} /></label>;
}
function money(value: number) { return new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 4 }).format(value); }
function tokens(value: number) { return new Intl.NumberFormat().format(value); }
function dateTime(timestamp: number) { return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(timestamp)); }
function Sparkline({ values }: { values: number[] }) {
  const max = Math.max(0, ...values);
  const points = values.map((value, index) => `${values.length < 2 ? 0 : index / (values.length - 1) * 100},${max === 0 ? 28 : 28 - value / max * 24}`).join(" ");
  return <svg className="usage-sparkline" viewBox="0 0 100 32" preserveAspectRatio="none" role="img" aria-label="Daily spend over the last 30 days"><polyline points={points} fill="none" stroke="currentColor" strokeWidth="1.5" vectorEffect="non-scaling-stroke" /></svg>;
}
