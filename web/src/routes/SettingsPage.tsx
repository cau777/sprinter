import { useEffect, useState } from "react";
import { Button, Input, TextArea } from "@heroui/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Check, CreditCard, FileText, KeyRound, LogOut, Save, Settings2, ShieldCheck, Star } from "lucide-react";
import { ApiError } from "../api/client";
import { fetchModels, fetchSessions, fetchSettings, fetchUsage, revokeAllSessions, revokeSession, updateSettingsAndRefetchFavorites } from "../api/settings";
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
    mutationFn: (patch: Parameters<typeof updateSettingsAndRefetchFavorites>[0]) => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to change settings.");
      return updateSettingsAndRefetchFavorites(patch);
    },
    onSuccess: (settings, patch) => {
      queryClient.setQueryData(["settings"], settings);
      setError(undefined);
      setSaved(patch.openrouter_api_key !== undefined ? "API key updated" : patch.custom_instructions !== undefined ? "Instructions saved" : "Settings saved");
      setApiKey("");
    },
    onError: (cause) => {
      setError(cause instanceof ApiError ? cause.message : "Could not save settings.");
    },
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
    <section className="mx-auto flex min-h-0 w-full max-w-[850px] flex-1 flex-col overflow-y-auto overscroll-contain px-7 pt-[31px] pb-[50px] max-[720px]:px-3.5 max-[720px]:pt-5 max-[720px]:pb-[calc(30px+env(safe-area-inset-bottom))]">
      <header className="flex items-center gap-[13px] border-b border-[var(--border)] pb-6 max-[720px]:pb-[18px]"><div className="grid size-[39px] shrink-0 place-items-center rounded-xl border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><Settings2 size={18} /></div><div><p className="mb-[3px] font-mono text-[11px] tracking-[.16em] text-[#8290a9]">YOUR WORKSPACE</p><h1 className="m-0 font-display text-[21px] font-medium text-[#eef3fa]">Settings</h1><p className="mt-0.5 mb-0 text-[11px] text-[#8290a6]">Keep your workspace tuned to the way you think.</p></div></header>
      {settingsQuery.isLoading ? <div className="text-[11px] text-[#7e8aa0]">Loading settings…</div> : settingsQuery.isError ? <div className="text-[11px] text-[#ff9aab]" role="alert">{settingsQuery.error.message}</div> : <>
        {!online && <div className="mt-3 rounded-lg border border-[var(--accent-line)] bg-[rgba(61,232,255,.035)] px-2.5 py-2 text-[11px] text-[#a8cbd3]" role="status">You’re offline. Settings are read-only until you reconnect.</div>}
        {error && <div className="mt-3 flex items-center gap-[7px] rounded-lg border border-[rgba(255,93,122,.2)] bg-[rgba(255,93,122,.04)] px-2.5 py-2 text-[11px] text-[#ff9aab]" role="alert">{error}</div>}
        {saved && <div className="mt-3 flex items-center gap-[7px] rounded-lg border border-[rgba(74,222,154,.18)] bg-[rgba(74,222,154,.035)] px-2.5 py-2 text-[11px] text-[#7cdaa9]" role="status"><Check size={14} />{saved}</div>}

        <fieldset disabled={!online} className="m-0 min-w-0 flex-1 border-0 p-0">
        <section className="pt-[21px]" aria-labelledby="provider-heading">
          <div className="mb-2.5 flex items-center gap-2.5"><div className="grid size-[30px] shrink-0 place-items-center rounded-[9px] border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><KeyRound size={16} /></div><div><h2 id="provider-heading" className="m-0 font-display text-xs font-medium text-[#dbe3ef]">OpenRouter</h2><p className="mt-0.5 mb-0 text-[11px] text-[#7f8ba1]">Your key connects Sprinter to model providers.</p></div></div>
          <div className="rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px]">
            <div className="flex items-center gap-[7px] text-[11px] text-[#95a1b6]"><span className={`size-1.5 rounded-full ${settings?.openrouter_api_key.set ? "bg-[var(--success)] shadow-[0_0_8px_rgba(74,222,154,.3)]" : "bg-[#758198]"}`} /><span>{settings?.openrouter_api_key.set ? settings.openrouter_api_key.readable ? "Key connected" : "Key needs to be entered again" : "No key saved yet"}</span>{settings?.openrouter_api_key.hint && <code className="ml-auto font-mono text-[11px] text-[#8b9ab1]">{settings.openrouter_api_key.hint}</code>}</div>
            <form className="mt-3.5 grid gap-[7px]" onSubmit={(event) => { event.preventDefault(); setSaved(undefined); setError(undefined); save.mutate({ openrouter_api_key: apiKey.trim() }); }}>
              <label className="text-[11px] font-semibold text-[#a8b4c7]" htmlFor="settings-openrouter-key">{settings?.openrouter_api_key.set ? "Replace API key" : "API key"}</label>
              <div className="flex gap-2 max-[720px]:flex-col"><Input id="settings-openrouter-key" className="h-9 min-w-0 flex-1 rounded-lg border border-[var(--field-border)] bg-[rgba(7,11,19,.46)] px-2.5 font-mono text-xs text-[var(--text-strong)] outline-none focus:border-[var(--accent-line)] focus:ring-2 focus:ring-[var(--accent-soft)]" type="password" autoComplete="off" placeholder="sk-or-v1-…" value={apiKey} onChange={(event) => setApiKey(event.target.value)} /><Button variant="primary" className="max-[720px]:w-full" isDisabled={!apiKey.trim() || save.isPending}>{save.isPending ? "Validating…" : "Validate and save"}</Button></div>
            </form>
            <p className="mt-[11px] mb-0 flex items-center gap-1.5 text-[11px] text-[#748198]"><ShieldCheck className="text-[#64c89a]" size={13} /> Encrypted at rest. The full key is never shown after saving.</p>
            {settings?.openrouter_api_key.set && <Button variant="danger" className="mt-2" onPress={() => { setSaved(undefined); setError(undefined); save.mutate({ openrouter_api_key: null }); }}>Remove saved key</Button>}
          </div>
        </section>

        <section className="pt-[21px]" aria-labelledby="models-heading">
          <div className="mb-2.5 flex items-center gap-2.5"><div className="grid size-[30px] shrink-0 place-items-center rounded-[9px] border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><Star size={16} /></div><div><h2 id="models-heading" className="m-0 font-display text-xs font-medium text-[#dbe3ef]">Models</h2><p className="mt-0.5 mb-0 text-[11px] text-[#7f8ba1]">Choose defaults and keep favorite models close.</p></div></div>
          {!settings?.openrouter_api_key.set ? <div className="rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px] text-[11px] text-[#7e8aa0]">Add an OpenRouter key to browse and set models.</div> : modelQuery.isLoading ? <div className="rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px] text-[11px] text-[#7e8aa0]">Loading available models…</div> : modelQuery.isError ? <div className="rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px] text-[11px] text-[#ff9aab]" role="alert">{modelQuery.error.message}</div> : <div className="grid gap-[17px] rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px]">
            <div className="grid grid-cols-2 gap-[15px] max-[720px]:grid-cols-1 max-[720px]:gap-[14px]">
              <label className="grid content-start gap-[7px] text-[11px] font-semibold text-[#a9b5c7]"><span>Default model</span><ModelPicker models={models} value={settings.default_model} favorites={favoriteModels} isDisabled={save.isPending} onChange={(default_model) => { setSaved(undefined); save.mutate({ default_model }); }} onToggleFavorite={(id) => changeFavorites(favoriteModels.includes(id) ? favoriteModels.filter((item) => item !== id) : [...favoriteModels, id])} /></label>
              <label className="grid content-start gap-[7px] text-[11px] font-semibold text-[#a9b5c7]"><span>Title model <small className="mt-[3px] block text-[11px] font-normal text-[#6f7d94]">Optional. Uses the conversation model when empty.</small></span><div className="flex items-center gap-[5px]"><ModelPicker className="min-w-0 flex-1" models={models} value={settings.title_model} favorites={favoriteModels} isDisabled={save.isPending} onChange={(title_model) => { setSaved(undefined); save.mutate({ title_model }); }} placeholder="Use conversation model" /><Button variant="secondary" isDisabled={save.isPending} onPress={() => { setSaved(undefined); save.mutate({ title_model: null }); }}>Clear</Button></div></label>
            </div>
            <div className="flex items-center justify-between font-mono text-[11px] text-[#8895aa]">FAVORITES <span className="font-sans text-[11px] text-[#6d7b92]">Shown first in the model picker</span></div>
            {favoriteModels.length === 0 ? <div className="py-[5px] text-[11px] text-[#6e7b91]">Star a model in a picker to add it here.</div> : <ul className="-mt-2.5 m-0 grid list-none p-0">{favoriteModels.map((id, index) => {
              const item = models.find((model) => model.id === id);
              return <li className="flex min-h-[38px] items-center gap-2 border-t border-[rgba(120,160,220,.07)] text-[#dfc76f]" key={id}><Star size={13} fill="currentColor" /><span className="flex-1 text-[11px] text-[#cbd4e2]">{item?.name ?? id}<small className="mt-0.5 block font-mono text-[11px] text-[#718098]">{id}</small></span><Button isIconOnly variant="ghost" className="h-7 w-7 text-slate-400" aria-label={`Move ${id} up`} isDisabled={index === 0} onPress={() => moveFavorite(index, -1)}><ArrowUp size={13} /></Button><Button isIconOnly variant="ghost" className="h-7 w-7 text-slate-400" aria-label={`Move ${id} down`} isDisabled={index === favoriteModels.length - 1} onPress={() => moveFavorite(index, 1)}><ArrowDown size={13} /></Button><Button variant="danger" onPress={() => changeFavorites(favoriteModels.filter((fav) => fav !== id))}>Remove</Button></li>;
            })}</ul>}
          </div>}
        </section>

        <section className="pt-[21px]" aria-labelledby="instructions-heading">
          <div className="mb-2.5 flex items-center gap-2.5"><div className="grid size-[30px] shrink-0 place-items-center rounded-[9px] border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><Save size={16} /></div><div><h2 id="instructions-heading" className="m-0 font-display text-xs font-medium text-[#dbe3ef]">Custom instructions</h2><p className="mt-0.5 mb-0 text-[11px] text-[#7f8ba1]">Context Sprinter will include in every conversation.</p></div></div>
          <div className="grid gap-2 rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px]">
            <label className="text-[11px] font-semibold text-[#a8b4c7]" htmlFor="custom-instructions">Instructions</label>
            <TextArea id="custom-instructions" className="min-h-28 w-full resize-y rounded-lg border border-[var(--field-border)] bg-[rgba(7,11,19,.4)] px-2.5 py-2 text-xs leading-7 text-[var(--text)] outline-none placeholder:text-slate-500 focus:border-[var(--accent-line)] focus:ring-2 focus:ring-[var(--accent-soft)]" value={instructions} onChange={(event) => setInstructions(event.target.value)} placeholder="For example: be concise, ask before making assumptions, and use metric units." rows={5} />
            <div className="flex items-center justify-between gap-2.5 max-[720px]:items-start max-[720px]:flex-col"><span className="text-[11px] text-[#6f7c91]">Sent as a private system instruction with each request.</span><Button variant="primary" className="max-[720px]:self-stretch" isDisabled={instructions === settings?.custom_instructions || save.isPending} onPress={() => { setSaved(undefined); setError(undefined); save.mutate({ custom_instructions: instructions }); }}>{save.isPending ? "Saving…" : "Save instructions"}</Button></div>
          </div>
        </section>

        <section className="pt-[21px]" aria-labelledby="files-heading">
          <div className="mb-2.5 flex items-center gap-2.5"><div className="grid size-[30px] shrink-0 place-items-center rounded-[9px] border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><FileText size={16} /></div><div><h2 id="files-heading" className="m-0 font-display text-xs font-medium text-[#dbe3ef]">Files</h2><p className="mt-0.5 mb-0 text-[11px] text-[#7f8ba1]">Set upload limits for attachments.</p></div></div>
          <div className="grid gap-4 rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px]">
            {uploadLimits && <>
              <div className="grid grid-cols-3 gap-3 max-[720px]:grid-cols-2">
                <LimitInput label="Images · MB per file" value={uploadLimits.image_bytes} scale={1024 * 1024} max={40} onChange={(n) => setUploadDraft({ ...uploadLimits, image_bytes: n })} />
                <LimitInput label="PDFs · MB per file" value={uploadLimits.pdf_bytes} scale={1024 * 1024} max={100} onChange={(n) => setUploadDraft({ ...uploadLimits, pdf_bytes: n })} />
                <LimitInput label="Text · MB per file" value={uploadLimits.text_bytes} scale={1024 * 1024} max={5} onChange={(n) => setUploadDraft({ ...uploadLimits, text_bytes: n })} />
                <LimitInput label="Files per message" value={uploadLimits.files_per_message} scale={1} max={20} onChange={(n) => setUploadDraft({ ...uploadLimits, files_per_message: n })} />
                <LimitInput label="Total prompt · MB" value={uploadLimits.total_prompt_bytes} scale={1024 * 1024} max={200} onChange={(n) => setUploadDraft({ ...uploadLimits, total_prompt_bytes: n })} />
              </div>
              <div className="flex items-center justify-between gap-3 border-t border-[rgba(120,160,220,.08)] pt-3 max-[720px]:items-stretch max-[720px]:flex-col"><small className="text-[11px] font-normal text-[#718098]">Server ceilings: images 40 MB, PDFs 100 MB, text 5 MB, 20 files, total 200 MB.</small><Button variant="primary" isDisabled={!uploadDraft || save.isPending} onPress={() => { setSaved(undefined); setError(undefined); save.mutate({ upload_limits: uploadDraft }, { onSuccess: () => setUploadDraft(undefined) }); }}>{save.isPending ? "Saving…" : "Save upload limits"}</Button></div>
            </>}
          </div>
        </section>

        <section className="pt-[21px]" aria-labelledby="usage-heading">
          <div className="mb-2.5 flex items-center gap-2.5"><div className="grid size-[30px] shrink-0 place-items-center rounded-[9px] border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><CreditCard size={16} /></div><div><h2 id="usage-heading" className="m-0 font-display text-xs font-medium text-[#dbe3ef]">Usage</h2><p className="mt-0.5 mb-0 text-[11px] text-[#7f8ba1]">Provider balance and recorded model spend.</p></div></div>
          {usageQuery.isLoading ? <div className="rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px] text-[11px] text-[#7e8aa0]">Loading usage…</div> : usageQuery.isError ? <div className="rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px] text-[11px] text-[#ff9aab]" role="alert">{usageQuery.error.message}</div> : usageQuery.data && <div className="grid gap-4 rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px]">
            <div className="grid grid-cols-[minmax(160px,.8fr)_2fr] gap-[18px] max-[720px]:grid-cols-1"><div className="grid content-start gap-1.5 rounded-[9px] border border-[rgba(61,232,255,.12)] bg-[rgba(61,232,255,.025)] px-3 py-2.5"><span className="font-mono text-[11px] uppercase text-[#7c8aa0]">OPENROUTER BALANCE</span><strong className="font-display text-xs font-medium text-[#d7e3f3]">{!settings?.openrouter_api_key.set ? "Set an API key to see your balance" : usageQuery.data.balance == null ? "Unavailable" : money(usageQuery.data.balance)}</strong></div><div className="grid grid-cols-4 gap-2 max-[720px]:grid-cols-2">{([["Today", usageQuery.data.totals.today], ["7 days", usageQuery.data.totals.d7], ["30 days", usageQuery.data.totals.d30], ["All time", usageQuery.data.totals.all]] as const).map(([label, total]) => <div className="grid content-start gap-[5px] border-l border-[rgba(120,160,220,.12)] px-2 py-[9px]" key={label}><span className="font-mono text-[11px] text-[#7c8aa0]">{label}</span><strong className="font-mono text-[11px] font-medium text-[#dce5f2]">{money(total.cost)}</strong><small className="text-[11px] text-[#718098]">{tokens(total.prompt_tokens + total.completion_tokens)} tokens</small></div>)}</div></div>
            <div className="grid grid-cols-[auto_1fr] items-center gap-x-4 gap-y-[3px] border-t border-[rgba(120,160,220,.08)] pt-3 pb-2 max-[720px]:grid-cols-1 max-[720px]:gap-[7px]"><div className="grid min-w-[102px] gap-[3px]"><strong className="text-[11px] text-[#bfcada]">Daily spend</strong><span className="text-[11px] text-[#718098]">Last 30 days · local time</span></div><Sparkline values={usageQuery.data.daily.map((item) => item.cost)} /><div className="col-start-2 flex justify-between text-[11px] text-[#718098] max-[720px]:col-start-1"><span>{usageQuery.data.daily[0]?.day ?? ""}</span><span>{usageQuery.data.daily.at(-1)?.day ?? ""}</span></div></div>
            <div className="grid grid-cols-2 gap-[18px] max-[720px]:grid-cols-1"><div className="min-w-0"><h3 className="mb-[7px] flex justify-between font-display text-[9px] font-medium text-[#aeb9ca]">By model</h3>{usageQuery.data.by_model.length ? <ul className="m-0 list-none p-0">{usageQuery.data.by_model.map((row) => <li className="flex min-h-[34px] min-w-0 items-center justify-between gap-2.5 border-t border-[rgba(120,160,220,.07)]" key={row.model}><span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] text-[#c2ccda]">{row.model}<small className="mt-0.5 block text-[11px] text-[#718098]">{tokens(row.prompt_tokens + row.completion_tokens)} tokens</small></span><strong className="shrink-0 font-mono text-[11px] font-medium text-[#dce5f2]">{money(row.cost)}</strong></li>)}</ul> : <p className="text-[11px] text-[#718098]">No recorded model usage yet.</p>}</div><div className="min-w-0"><h3 className="mb-[7px] flex justify-between font-display text-[9px] font-medium text-[#aeb9ca]">By chat <small className="font-sans text-[11px] text-[#718098]">Top 20</small></h3>{usageQuery.data.by_chat.length ? <ul className="m-0 list-none p-0">{usageQuery.data.by_chat.map((row) => <li className="flex min-h-[34px] min-w-0 items-center justify-between gap-2.5 border-t border-[rgba(120,160,220,.07)]" key={row.chat_id}><span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[11px] text-[#c2ccda]" title={row.chat_title || "Untitled chat"}>{row.chat_title || "Untitled chat"}{row.title_cost > 0 && <small className="mt-0.5 block text-[11px] text-[#718098]">Includes {money(row.title_cost)} title generation</small>}</span><strong className="shrink-0 font-mono text-[11px] font-medium text-[#dce5f2]">{money(row.cost)}</strong></li>)}</ul> : <p className="text-[11px] text-[#718098]">No recorded chat usage yet.</p>}</div></div>
            <p className="m-0 border-t border-[rgba(120,160,220,.07)] pt-[9px] text-[11px] text-[#718098]">Spend totals use recorded message costs. Messages without provider cost data may be omitted.</p>
          </div>}
        </section>

        <section className="pt-[21px]" aria-labelledby="sessions-heading">
          <div className="mb-2.5 flex items-center gap-2.5"><div className="grid size-[30px] shrink-0 place-items-center rounded-[9px] border border-[var(--accent-line)] bg-[var(--accent-soft)] text-[var(--accent)]"><LogOut size={16} /></div><div><h2 id="sessions-heading" className="m-0 font-display text-xs font-medium text-[#dbe3ef]">Active sessions</h2><p className="mt-0.5 mb-0 text-[11px] text-[#7f8ba1]">Devices currently signed in to your workspace.</p></div></div>
          {sessionsQuery.isLoading ? <div className="rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px] text-[11px] text-[#7e8aa0]">Loading sessions…</div> : sessionsQuery.isError ? <div className="rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px] text-[11px] text-[#ff9aab]" role="alert">{sessionsQuery.error.message}</div> : <div className="grid gap-4 rounded-xl border border-[var(--border)] bg-[rgba(16,23,37,.56)] p-[15px]">
            <div className="flex items-center justify-between text-[11px] text-[#8b98ad]"><span>{sessionsQuery.data?.length ?? 0} active {(sessionsQuery.data?.length ?? 0) === 1 ? "session" : "sessions"}</span><Button variant="danger" isDisabled={!sessionsQuery.data?.length || revokeEvery.isPending} onPress={() => revokeEvery.mutate()}>{revokeEvery.isPending ? "Logging out…" : "Log out all sessions"}</Button></div>
            {!sessionsQuery.data?.length ? <p className="text-[11px] text-[#7e8aa0]">No active sessions.</p> : <ul className="m-0 list-none p-0">{sessionsQuery.data.map((session) => <li className="flex min-h-[53px] items-center justify-between gap-3 border-t border-[rgba(120,160,220,.07)]" key={session.id}><span className="min-w-0 break-words text-[11px] text-[#c3cedd]">{session.user_agent || "Unknown device"}{session.current && <b className="ml-[7px] inline-block rounded bg-[rgba(61,232,255,.08)] px-1 py-0.5 font-mono text-[11px] text-[var(--accent)]">THIS DEVICE</b>}<small className="mt-[3px] block text-[11px] text-[#718098]">{session.ip || "Unknown address"} · Signed in {dateTime(session.created_at)} · Last active {dateTime(session.last_seen_at)}</small></span><Button variant="danger" isDisabled={revokeOne.isPending} onPress={() => revokeOne.mutate(session.id)}>{session.current ? "Log out" : "Revoke"}</Button></li>)}</ul>}
          </div>}
        </section>
        </fieldset>
      </>}
    </section>
  );
}

function LimitInput({ label, value, scale, max, onChange }: { label: string; value: number; scale: number; max: number; onChange: (value: number) => void }) {
  const shown = value / scale;
  return <label className="grid gap-[6px] text-[11px] font-semibold text-[#a9b5c7]"><span>{label}</span><Input className="h-9 w-full rounded-lg border border-[var(--field-border)] bg-[rgba(7,11,19,.46)] px-2 text-xs text-[var(--text)] outline-none focus:border-[var(--accent-line)]" type="number" min={scale === 1 ? 1 : 0.01} max={max} step={scale === 1 ? 1 : 0.1} value={Number.isInteger(shown) ? shown : Number(shown.toFixed(2))} onChange={(event) => { const parsed = Number(event.target.value); if (Number.isFinite(parsed)) onChange(Math.round(parsed * scale)); }} /></label>;
}
function money(value: number) { return new Intl.NumberFormat(undefined, { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 4 }).format(value); }
function tokens(value: number) { return new Intl.NumberFormat().format(value); }
function dateTime(timestamp: number) { return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(timestamp)); }
function Sparkline({ values }: { values: number[] }) {
  const max = Math.max(0, ...values);
  const points = values.map((value, index) => `${values.length < 2 ? 0 : index / (values.length - 1) * 100},${max === 0 ? 28 : 28 - value / max * 24}`).join(" ");
  return <svg className="block h-[54px] w-full overflow-visible text-[var(--accent)]" viewBox="0 0 100 32" preserveAspectRatio="none" role="img" aria-label="Daily spend over the last 30 days"><polyline points={points} fill="none" stroke="currentColor" strokeWidth="1.5" vectorEffect="non-scaling-stroke" /></svg>;
}
