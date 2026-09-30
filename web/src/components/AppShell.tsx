import { useEffect, useState } from "react";
import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { Button, Input, Modal } from "@heroui/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { CircleHelp, LogOut, Menu, MessageSquarePlus, Pencil, Search, Settings2, Sparkles, Trash2, X } from "lucide-react";
import { apiRequest } from "../api/client";
import { deleteChat, fetchChat, fetchChats, updateChat } from "../api/chats";
import type { ChatDetail, ChatPage } from "../api/chats";
import { clearPersistedQueryCache } from "../api/queryPersistence";
import { useOnlineStatus } from "../api/useOnlineStatus";
import { fetchModels, fetchSettings } from "../api/settings";
import { ChatExportMenu } from "./ChatExportMenu";
import { ModelPicker } from "./ModelPicker";
import { SearchDialog } from "./SearchDialog";
import { HelpDialog } from "./HelpDialog";
import { useRegisterSW } from "virtual:pwa-register/react";
import packageJson from "../../package.json";

export function AppShell() {
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [searchOpen, setSearchOpen] = useState(false);
  const [helpOpen, setHelpOpen] = useState(false);
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
    <main className="flex h-dvh gap-3 overflow-hidden p-3 max-[720px]:gap-0 max-[720px]:p-0">
      <aside className="flex h-[calc(100dvh-24px)] min-h-0 w-[250px] shrink-0 flex-col rounded-[18px] border border-[var(--border)] bg-[var(--panel)] px-[13px] pt-5 pb-3 backdrop-blur-[18px] max-[720px]:hidden">
        <SidebarContents online={online} onNavigate={() => setDrawerOpen(false)} onSearch={() => setSearchOpen(true)} onHelp={() => setHelpOpen(true)} />
      </aside>

      {drawerOpen && <div className="fixed inset-0 z-20 bg-[rgba(1,4,10,.65)] backdrop-blur-[3px]" onClick={() => setDrawerOpen(false)}>
        <aside className="absolute inset-y-2 left-2 z-[21] flex w-[min(285px,calc(100vw-40px))] min-h-0 flex-col rounded-[18px] border border-[var(--border)] bg-[var(--panel)] px-[13px] pt-5 pb-3 shadow-[20px_0_70px_rgba(0,0,0,.32)] backdrop-blur-[18px] animate-[drawer-in_.17s_ease-out]" onClick={(event) => event.stopPropagation()}>
          <Button isIconOnly className="absolute right-3 top-4 z-10 hidden h-7 w-7 text-slate-400 max-[720px]:flex" variant="ghost" aria-label="Close navigation" onPress={() => setDrawerOpen(false)}><X size={17} /></Button>
          <SidebarContents online={online} onNavigate={() => setDrawerOpen(false)} onSearch={() => { setDrawerOpen(false); setSearchOpen(true); }} onHelp={() => { setDrawerOpen(false); setHelpOpen(true); }} />
        </aside>
      </div>}

      <section className="flex h-[calc(100dvh-24px)] min-h-0 min-w-0 flex-1 flex-col overflow-hidden max-[720px]:h-dvh">
        {!online && <div className="flex items-center justify-center gap-3 border-b border-amber-300/20 bg-amber-300/10 px-3.5 py-2 text-center text-[11px] text-amber-200" role="status">You’re offline. Saved conversations are available to read.</div>}
        {needRefresh && !generationOpen && <div className="flex items-center justify-center gap-3 border-b border-cyan-300/20 bg-cyan-300/10 px-3.5 py-2 text-center text-[11px] text-cyan-100" role="status">
          <span>A Sprinter update is ready.</span>
          <Button variant="secondary" onPress={() => void updateServiceWorker(true)}>Reload to update</Button>
        </div>}
        <header className="flex h-[55px] shrink-0 items-center justify-between border-b border-[rgba(120,160,220,.08)] px-[23px] max-[720px]:h-[49px] max-[720px]:px-[15px]">
          <div className="flex min-w-0 flex-1 items-center gap-[11px]">
            <Button isIconOnly className="-ml-2 hidden h-8 w-8 text-slate-400 max-[720px]:grid" variant="ghost" aria-label="Open navigation" onPress={() => setDrawerOpen(true)}><Menu size={17} /></Button>
            <span className="font-mono text-[11px] tracking-[.1em] text-slate-500 max-[720px]:hidden">WORKSPACE</span><span className="text-slate-700 max-[720px]:hidden">/</span><span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap text-xs text-slate-300">{pageName}</span>
          </div>
          <div className="flex items-center gap-2 text-[11px] text-slate-500 max-[720px]:gap-1.5 max-[720px]:text-[11px]">
            {activeChatId && modelsQuery.data && settingsQuery.data && <div className="w-[260px] min-w-0 max-[720px]:w-[min(190px,48vw)]"><ModelPicker compact models={modelsQuery.data.items} value={currentChatQuery.data?.model ?? currentChat?.model ?? null} favorites={settingsQuery.data.favorite_models} isDisabled={!online || modelChange.isPending} onChange={(model) => modelChange.mutate({ chatId: activeChatId, model })} placeholder="Choose model" /></div>}
            {activeChatId && <ChatExportMenu chatId={activeChatId} />}
          </div>
        </header>
        <Outlet />
      </section>
      <SearchDialog open={searchOpen} onClose={() => setSearchOpen(false)} />
      <HelpDialog open={helpOpen} onClose={() => setHelpOpen(false)} />
    </main>
  );
}

function SidebarContents({ online, onNavigate, onSearch, onHelp }: { online: boolean; onNavigate: () => void; onSearch: () => void; onHelp: () => void }) {
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
      <span className="flex flex-col gap-1"><span>SPRINTER</span><span className="font-mono text-[11px] font-normal tracking-[.08em] text-slate-500">v{packageJson.version}</span></span>
    </Link>

    <div className="grid gap-[7px]">
      <Link to="/" className="flex h-10 w-full items-center gap-2.5 rounded-xl border border-[var(--accent-line)] bg-[rgba(61,232,255,.025)] px-3 text-xs text-[var(--accent)] no-underline" onClick={onNavigate}>
        <MessageSquarePlus size={16} /><span>New chat</span><kbd>⌘ ⇧ O</kbd>
      </Link>
      <Button className="flex h-9 w-full items-center justify-start gap-2.5 rounded-lg border-0 bg-transparent px-2 text-left text-[13px] font-medium leading-[1.2] text-[#b0bdcf] no-underline transition-colors hover:bg-white/[.05] hover:text-[#f1f5f9] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" variant="ghost" onPress={onSearch}><Search size={16} /><span>Search chats</span><kbd className="ml-auto rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[11px] text-slate-500">⌘ K</kbd></Button>
    </div>

    <div className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-2 pt-[30px] pb-3">
      <div className="mb-3 font-mono text-[11px] tracking-[.09em] text-slate-600">YOUR SPACE</div>
      <ConversationList online={online} onNavigate={onNavigate} />
    </div>

    <div className="mt-auto grid gap-1">
      <Link to="/settings" className="flex h-9 w-full items-center gap-2.5 rounded-lg px-2 text-[13px] font-medium leading-[1.2] text-[#b0bdcf] no-underline transition-colors hover:bg-white/[.05] hover:text-[#f1f5f9] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" onClick={onNavigate}><Settings2 size={16} /><span>Settings</span></Link>
      <Button variant="ghost" className="flex h-9 w-full items-center justify-start gap-2.5 rounded-lg border-0 bg-transparent px-2 text-left text-[13px] font-medium leading-[1.2] text-[#b0bdcf] no-underline transition-colors hover:bg-white/[.05] hover:text-[#f1f5f9] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" onPress={onHelp}><CircleHelp size={16} /><span>Help & shortcuts</span></Button>
      <Button variant="ghost" className="flex h-9 w-full items-center justify-start gap-2.5 rounded-lg border-0 bg-transparent px-2 text-left text-[13px] font-medium leading-[1.2] text-[#b0bdcf] no-underline transition-colors hover:bg-white/[.05] hover:text-[#f1f5f9] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)]" onPress={() => void logout()}><LogOut size={15} /><span>Log out</span></Button>
    </div>
  </>;
}

function ConversationList({ online, onNavigate }: { online: boolean; onNavigate: () => void }) {
  const query = useQuery({ queryKey: ["chats"], queryFn: fetchChats });
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const [renameTarget, setRenameTarget] = useState<{ id: string; title: string | null } | null>(null);
  const [renameTitle, setRenameTitle] = useState("");
  const [deleteTarget, setDeleteTarget] = useState<{ id: string; title: string | null } | null>(null);
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
  return <><div className="grid gap-[15px]">{["Today", "Previous 7 days", "Earlier"].map((group) => {
    const items = groups.get(group);
    return items?.length ? <div className="grid gap-0.5" key={group}><div className="px-px pt-0.5 pb-1.5 font-mono text-[11px] uppercase tracking-[.11em] text-slate-600">{group}</div>{items.map((chat) => <div key={chat.id} className="group flex min-w-0 items-center rounded-md hover:bg-white/[.035]"><Link to="/$chatId" params={{ chatId: chat.id }} search={{ messageId: undefined }} className="block min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap px-0.5 py-1.5 pl-[7px] text-[11px] text-slate-400 no-underline hover:text-slate-100" onClick={onNavigate}>{chat.title || "New conversation"}</Link><div className="flex gap-px opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 max-[720px]:opacity-100"><Button isIconOnly variant="ghost" className="h-6 w-6 rounded-md text-slate-500 hover:text-[var(--accent)]" aria-label={`Rename ${chat.title || "conversation"}`} isDisabled={!online} onPress={() => { setRenameTarget(chat); setRenameTitle(chat.title || ""); }}><Pencil size={12} /></Button><Button isIconOnly variant="ghost" className="h-6 w-6 rounded-md text-slate-500 hover:text-[var(--danger)]" aria-label={`Delete ${chat.title || "conversation"}`} isDisabled={!online} onPress={() => setDeleteTarget(chat)}><Trash2 size={12} /></Button></div></div>)}</div> : null;
  })}</div>
    <Modal isOpen={Boolean(renameTarget)} onOpenChange={(isOpen) => { if (!isOpen && !rename.isPending) setRenameTarget(null); }}>
      <Modal.Backdrop className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm"><Modal.Container className="fixed inset-0 z-50 flex w-full items-center justify-center p-3" placement="center"><Modal.Dialog aria-labelledby="rename-chat-title" className="w-[min(420px,calc(100vw-28px))] rounded-2xl border border-[var(--border)] bg-[#0d1421] text-[var(--text)] shadow-2xl"><Modal.Header className="px-4 pt-4 pb-2"><Modal.Heading id="rename-chat-title" className="font-display text-lg font-medium text-slate-100">Rename conversation</Modal.Heading></Modal.Header><Modal.Body className="px-4 pb-3"><Input autoFocus aria-label="Conversation title" className="w-full rounded-lg border border-[var(--field-border)] bg-[rgba(7,11,19,.4)] px-3 py-2 text-sm text-[var(--text)] outline-none" value={renameTitle} onChange={(event) => setRenameTitle(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter" && renameTarget && renameTitle.trim()) rename.mutate({ id: renameTarget.id, title: renameTitle.trim() }, { onSuccess: () => setRenameTarget(null) }); }} />{rename.isError && <p className="mt-2 mb-0 text-[11px] text-rose-300" role="alert">Couldn’t rename this conversation.</p>}</Modal.Body><Modal.Footer className="flex justify-end gap-2 border-t border-[var(--border)] px-4 py-2"><Button variant="ghost" onPress={() => setRenameTarget(null)} isDisabled={rename.isPending}>Cancel</Button><Button variant="primary" isDisabled={!renameTitle.trim() || rename.isPending} onPress={() => { if (renameTarget) rename.mutate({ id: renameTarget.id, title: renameTitle.trim() }, { onSuccess: () => setRenameTarget(null) }); }}>{rename.isPending ? "Saving…" : "Save"}</Button></Modal.Footer></Modal.Dialog></Modal.Container></Modal.Backdrop>
    </Modal>
    <Modal isOpen={Boolean(deleteTarget)} onOpenChange={(isOpen) => { if (!isOpen && !remove.isPending) setDeleteTarget(null); }}>
      <Modal.Backdrop className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm"><Modal.Container className="fixed inset-0 z-50 flex w-full items-center justify-center p-3" placement="center"><Modal.Dialog aria-labelledby="delete-chat-title" className="w-[min(420px,calc(100vw-28px))] rounded-2xl border border-[var(--border)] bg-[#0d1421] text-[var(--text)] shadow-2xl"><Modal.Header className="px-4 pt-4 pb-2"><Modal.Heading id="delete-chat-title" className="font-display text-lg font-medium text-slate-100">Delete conversation?</Modal.Heading></Modal.Header><Modal.Body className="px-4 pb-3"><p className="m-0 text-sm leading-6 text-slate-300">This permanently deletes {deleteTarget?.title ? `“${deleteTarget.title}”` : "this conversation"} and its messages.</p>{remove.isError && <p className="mt-2 mb-0 text-[11px] text-rose-300" role="alert">Couldn’t delete this conversation.</p>}</Modal.Body><Modal.Footer className="flex justify-end gap-2 border-t border-[var(--border)] px-4 py-2"><Button variant="ghost" onPress={() => setDeleteTarget(null)} isDisabled={remove.isPending}>Cancel</Button><Button variant="danger" isDisabled={remove.isPending} onPress={() => { if (deleteTarget) remove.mutate(deleteTarget.id, { onSuccess: () => setDeleteTarget(null) }); }}>{remove.isPending ? "Deleting…" : "Delete"}</Button></Modal.Footer></Modal.Dialog></Modal.Container></Modal.Backdrop>
    </Modal>
  </>;
}
