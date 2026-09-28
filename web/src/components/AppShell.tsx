import { useEffect, useState } from "react";
import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { Button } from "@heroui/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleHelp, Command, LogOut, Menu, MessageSquarePlus, Pencil, Search, Settings2, Sparkles, Trash2, X } from "lucide-react";
import { apiRequest } from "../api/client";
import { deleteChat, fetchChat, fetchChats, updateChat } from "../api/chats";
import type { ChatDetail, ChatPage } from "../api/chats";
import { clearPersistedQueryCache } from "../api/queryPersistence";
import { useOnlineStatus } from "../api/useOnlineStatus";
import { fetchModels, fetchSettings } from "../api/settings";
import { ChatExportMenu } from "./ChatExportMenu";
import { ModelPicker } from "./ModelPicker";
import { SearchDialog } from "./SearchDialog";
import { useRegisterSW } from "virtual:pwa-register/react";

export function AppShell() {
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const online = useOnlineStatus();
  const { needRefresh: [needRefresh], updateServiceWorker } = useRegisterSW();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const currentPath = useRouterState({ select: (state) => state.location.pathname });
  const chatList = useQuery({ queryKey: ["chats"], queryFn: fetchChats });
  const currentChat = chatList.data?.items.find((chat) => `/${chat.id}` === currentPath);
  const routeChatId = currentPath === "/" || currentPath === "/settings" || currentPath === "/setup" || currentPath === "/spike/assistant-ui" ? undefined : currentPath.slice(1);
  const activeChatId = currentChat?.id ?? routeChatId;
  const currentChatQuery = useQuery({ queryKey: ["chat", activeChatId], queryFn: () => fetchChat(activeChatId!), enabled: Boolean(activeChatId) });
  const modelsQuery = useQuery({ queryKey: ["models"], queryFn: fetchModels, enabled: Boolean(activeChatId) });
  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: fetchSettings, enabled: Boolean(activeChatId) });
  const modelChange = useMutation({
    mutationFn: ({ chatId, model }: { chatId: string; model: string }) => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to change the model.");
      return updateChat(chatId, { model });
    },
    onSuccess: (summary) => {
      queryClient.setQueryData<ChatPage>(["chats"], (current) => current ? { ...current, items: current.items.map((chat) => chat.id === summary.id ? summary : chat) } : current);
      queryClient.setQueryData<ChatDetail>(["chat", summary.id], (current) => current ? { ...current, model: summary.model } : current);
      void queryClient.invalidateQueries({ queryKey: ["chat", summary.id] });
    },
  });
  const generationOpen = currentChatQuery.data?.messages.some((message) => message.status === "streaming") ?? false;
  const pageName = currentPath === "/settings" ? "Settings" : currentPath === "/setup" ? "Workspace setup" : currentPath === "/spike/assistant-ui" ? "Runtime lab" : currentChatQuery.data?.title || currentChat?.title || (activeChatId ? "Conversation" : currentPath === "/" ? "New conversation" : "Conversation");

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (!(event.metaKey || event.ctrlKey) || event.altKey) return;
      if (event.key.toLowerCase() === "k" && !event.shiftKey) {
        event.preventDefault();
        setSearchOpen(true);
      } else if (event.key.toLowerCase() === "o" && event.shiftKey) {
        event.preventDefault();
        void navigate({ to: "/" });
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [navigate]);

  return (
    <main className="flex min-h-dvh gap-3 p-3 max-[720px]:gap-0 max-[720px]:p-0">
      <aside className="flex min-h-[calc(100dvh-24px)] w-[250px] shrink-0 flex-col rounded-[18px] border border-[var(--border)] bg-[var(--panel)] px-[13px] pt-5 pb-3 backdrop-blur-[18px] max-[720px]:hidden">
        <SidebarContents online={online} onNavigate={() => setDrawerOpen(false)} onSearch={() => setSearchOpen(true)} />
      </aside>

      {drawerOpen && <div className="fixed inset-0 z-20 bg-[rgba(1,4,10,.65)] backdrop-blur-[3px]" onClick={() => setDrawerOpen(false)}>
        <aside className="absolute inset-y-2 left-2 z-[21] flex w-[min(285px,calc(100vw-40px))] min-h-0 flex-col rounded-[18px] border border-[var(--border)] bg-[var(--panel)] px-[13px] pt-5 pb-3 shadow-[20px_0_70px_rgba(0,0,0,.32)] backdrop-blur-[18px] animate-[drawer-in_.17s_ease-out]" onClick={(event) => event.stopPropagation()}>
          <Button isIconOnly className="absolute right-3 top-4 z-10 hidden h-7 w-7 text-slate-400 max-[720px]:flex" variant="ghost" aria-label="Close navigation" onPress={() => setDrawerOpen(false)}><X size={17} /></Button>
          <SidebarContents online={online} onNavigate={() => setDrawerOpen(false)} onSearch={() => { setDrawerOpen(false); setSearchOpen(true); }} />
        </aside>
      </div>}

      <section className="flex min-h-[calc(100dvh-24px)] min-w-0 flex-1 flex-col max-[720px]:min-h-dvh">
        {!online && <div className="flex items-center justify-center gap-3 border-b border-amber-300/20 bg-amber-300/10 px-3.5 py-2 text-center text-[11px] text-amber-200" role="status">You’re offline. Saved conversations are available to read.</div>}
        {needRefresh && !generationOpen && <div className="flex items-center justify-center gap-3 border-b border-cyan-300/20 bg-cyan-300/10 px-3.5 py-2 text-center text-[11px] text-cyan-100" role="status">
          <span>A Sprinter update is ready.</span>
          <Button variant="ghost" className="rounded-md border border-[var(--accent-line)] bg-[var(--accent-soft)] px-2 py-1 text-[10px] text-[var(--accent)]" onPress={() => void updateServiceWorker(true)}>Reload to update</Button>
        </div>}
        <header className="flex h-[55px] shrink-0 items-center justify-between border-b border-[rgba(120,160,220,.08)] px-[23px] max-[720px]:h-[49px] max-[720px]:px-[15px]">
          <div className="flex min-w-0 flex-1 items-center gap-[11px]">
            <Button isIconOnly className="-ml-2 hidden h-8 w-8 text-slate-400 max-[720px]:grid" variant="ghost" aria-label="Open navigation" onPress={() => setDrawerOpen(true)}><Menu size={17} /></Button>
            <span className="font-mono text-[9px] tracking-[.1em] text-slate-500 max-[720px]:hidden">WORKSPACE</span><span className="text-slate-700 max-[720px]:hidden">/</span><span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap text-xs text-slate-300">{pageName}</span>
          </div>
          <div className="flex items-center gap-2 text-[10px] text-slate-500 max-[720px]:gap-1.5 max-[720px]:text-[9px]">
            {activeChatId && modelsQuery.data && settingsQuery.data && <div className="w-[190px] min-w-0 max-[720px]:w-[min(145px,37vw)]"><ModelPicker compact models={modelsQuery.data.items} value={currentChatQuery.data?.model ?? currentChat?.model ?? null} favorites={settingsQuery.data.favorite_models} isDisabled={!online || modelChange.isPending} onChange={(model) => modelChange.mutate({ chatId: activeChatId, model })} placeholder="Choose model" /></div>}
            {activeChatId && <ChatExportMenu chatId={activeChatId} />}
            <span className="size-1.5 rounded-full bg-emerald-400 shadow-[0_0_8px_rgba(74,222,154,.35)] max-[720px]:hidden" /> <span className="max-[720px]:hidden">Local and private</span>
          </div>
        </header>
        <Outlet />
      </section>
      <SearchDialog open={searchOpen} onClose={() => setSearchOpen(false)} />
    </main>
  );
}

function SidebarContents({ online, onNavigate, onSearch }: { online: boolean; onNavigate: () => void; onSearch: () => void }) {
  const queryClient = useQueryClient();
  async function logout() {
    await apiRequest("/api/auth/logout", { method: "POST", body: "{}" }).catch(() => undefined);
    const clearingCache = clearPersistedQueryCache().catch(() => undefined);
    queryClient.clear();
    await clearingCache;
    window.location.assign("/login");
  }

  return <>
    <Link to="/" className="flex items-center gap-2.5 px-[9px] pt-0.5 pb-[23px] font-display text-[15px] font-bold leading-none tracking-[.16em] text-white no-underline" aria-label="Sprinter home" onClick={onNavigate}>
      <span className="grid size-[29px] place-items-center rounded-[9px] bg-[var(--accent)] text-[var(--bg)] shadow-[0_0_22px_rgba(61,232,255,.2)]"><Sparkles size={17} strokeWidth={1.8} /></span>
      <span>SPRINTER</span>
    </Link>

    <div className="grid gap-[7px]">
      <Link to="/" className="flex h-10 w-full items-center gap-2.5 rounded-xl border border-[var(--accent-line)] bg-[rgba(61,232,255,.025)] px-3 text-xs text-[var(--accent)] no-underline" onClick={onNavigate}>
        <MessageSquarePlus size={16} /><span>New chat</span><kbd>⌘ ⇧ O</kbd>
      </Link>
      <Button className="h-10 w-full justify-start gap-2.5 rounded-xl px-3 text-xs text-slate-400" variant="ghost" onPress={onSearch}><Search size={16} /><span>Search chats</span><kbd className="ml-auto rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[10px] text-slate-500">⌘ K</kbd></Button>
    </div>

    <div className="px-2 pt-[30px] pb-3">
      <div className="mb-3 font-mono text-[10px] tracking-[.09em] text-slate-600">YOUR SPACE</div>
      <ConversationList online={online} onNavigate={onNavigate} />
    </div>

    <div className="mt-auto grid gap-1">
      <Link to="/settings" className="flex items-center gap-2.5 rounded-lg px-2 py-2 text-xs text-slate-400 no-underline hover:bg-white/5 hover:text-slate-100" onClick={onNavigate}><Settings2 size={16} /><span>Settings</span></Link>
      <Button variant="ghost" className="justify-start gap-2.5 rounded-lg px-2 py-2 text-left text-xs text-slate-400 hover:bg-white/5 hover:text-slate-100"><CircleHelp size={16} /><span>Help & shortcuts</span></Button>
      <div className="mt-[9px] flex items-center gap-2.5 border-t border-[var(--border)] px-[5px] pt-[15px] pb-1"><div className="grid size-[30px] place-items-center rounded-[10px] bg-[linear-gradient(145deg,#26354d,#152032)] font-display text-xs font-semibold text-slate-300">S</div><div><div className="text-[11px] font-semibold text-slate-200">Sprinter</div><div className="text-[10px] text-slate-500">Private workspace</div></div><Command className="ml-auto text-slate-500" size={15} /></div>
      <Button variant="ghost" className="w-full justify-start gap-2.5 rounded-lg px-2 py-2 text-left text-xs text-slate-500 hover:bg-white/5 hover:text-slate-100" onPress={() => void logout()}><LogOut size={15} /><span>Log out</span></Button>
    </div>
  </>;
}

function ConversationList({ online, onNavigate }: { online: boolean; onNavigate: () => void }) {
  const query = useQuery({ queryKey: ["chats"], queryFn: fetchChats });
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const rename = useMutation({ mutationFn: ({ id, title }: { id: string; title: string }) => {
    if (!navigator.onLine) throw new Error("You’re offline. Reconnect to rename conversations.");
    return updateChat(id, { title });
  }, onSuccess: () => queryClient.invalidateQueries({ queryKey: ["chats"] }) });
  const remove = useMutation({ mutationFn: (id: string) => {
    if (!navigator.onLine) throw new Error("You’re offline. Reconnect to delete conversations.");
    return deleteChat(id);
  }, onSuccess: (_, id) => { queryClient.invalidateQueries({ queryKey: ["chats"] }); queryClient.removeQueries({ queryKey: ["chat", id] }); if (window.location.pathname === `/${id}`) void navigate({ to: "/" }); } });
  const chats = query.data?.items ?? [];
  if (query.isLoading) return <div className="flex gap-2 px-px py-2 text-[11px] leading-snug text-slate-500"><span className="mt-1.5 size-[5px] shrink-0 rounded-full bg-slate-600" />Loading conversations…</div>;
  if (!chats.length) return <div className="flex gap-2 px-px py-2 text-[11px] leading-snug text-slate-500"><span className="mt-1.5 size-[5px] shrink-0 rounded-full bg-slate-600" />Your conversations will appear here.</div>;
  const groups = new Map<string, typeof chats>();
  for (const chat of chats) {
    const age = Date.now() - chat.updated_at;
    const group = age < 86_400_000 ? "Today" : age < 7 * 86_400_000 ? "Previous 7 days" : "Earlier";
    groups.set(group, [...(groups.get(group) ?? []), chat]);
  }
  return <div className="grid gap-[15px]">{["Today", "Previous 7 days", "Earlier"].map((group) => {
    const items = groups.get(group);
    return items?.length ? <div className="grid gap-0.5" key={group}><div className="px-px pt-0.5 pb-1.5 font-mono text-[8px] uppercase tracking-[.11em] text-slate-600">{group}</div>{items.map((chat) => <div key={chat.id} className="group flex min-w-0 items-center rounded-md hover:bg-white/[.035]"><Link to="/$chatId" params={{ chatId: chat.id }} search={{ messageId: undefined }} className="block min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap px-0.5 py-1.5 pl-[7px] text-[10px] text-slate-400 no-underline hover:text-slate-100" onClick={onNavigate}>{chat.title || "New conversation"}</Link><div className="flex gap-px opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 max-[720px]:opacity-100"><Button isIconOnly variant="ghost" className="h-6 w-6 rounded-md text-slate-500 hover:text-[var(--accent)]" aria-label={`Rename ${chat.title || "conversation"}`} isDisabled={!online} onPress={() => { const title = window.prompt("Rename conversation", chat.title || ""); if (title?.trim()) rename.mutate({ id: chat.id, title: title.trim() }); }}><Pencil size={12} /></Button><Button isIconOnly variant="ghost" className="h-6 w-6 rounded-md text-slate-500 hover:text-[var(--danger)]" aria-label={`Delete ${chat.title || "conversation"}`} isDisabled={!online} onPress={() => { if (window.confirm("Delete this conversation?")) remove.mutate(chat.id); }}><Trash2 size={12} /></Button></div></div>)}</div> : null;
  })}</div>;
}
