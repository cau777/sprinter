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
    <main className="app-frame">
      <aside className="sidebar glass-panel desktop-sidebar">
        <SidebarContents online={online} onNavigate={() => setDrawerOpen(false)} onSearch={() => setSearchOpen(true)} />
      </aside>

      {drawerOpen && <div className="mobile-drawer-scrim" onClick={() => setDrawerOpen(false)}>
        <aside className="sidebar glass-panel mobile-sidebar" onClick={(event) => event.stopPropagation()}>
          <Button isIconOnly className="drawer-close" variant="ghost" aria-label="Close navigation" onPress={() => setDrawerOpen(false)}><X size={17} /></Button>
          <SidebarContents online={online} onNavigate={() => setDrawerOpen(false)} onSearch={() => { setDrawerOpen(false); setSearchOpen(true); }} />
        </aside>
      </div>}

      <section className="main-panel">
        {!online && <div className="pwa-status-banner" role="status">You’re offline. Saved conversations are available to read.</div>}
        {needRefresh && !generationOpen && <div className="pwa-status-banner pwa-update-banner" role="status">
          <span>A Sprinter update is ready.</span>
          <button type="button" onClick={() => void updateServiceWorker(true)}>Reload to update</button>
        </div>}
        <header className="topbar">
          <div className="breadcrumb">
            <Button isIconOnly className="mobile-menu-button" variant="ghost" aria-label="Open navigation" onPress={() => setDrawerOpen(true)}><Menu size={17} /></Button>
            <span className="topbar-kicker">WORKSPACE</span><span className="breadcrumb-slash">/</span><span className="breadcrumb-current">{pageName}</span>
          </div>
          <div className="topbar-right">
            {activeChatId && modelsQuery.data && settingsQuery.data && <div className="topbar-model-picker"><ModelPicker models={modelsQuery.data.items} value={currentChatQuery.data?.model ?? currentChat?.model ?? null} favorites={settingsQuery.data.favorite_models} isDisabled={!online || modelChange.isPending} onChange={(model) => modelChange.mutate({ chatId: activeChatId, model })} placeholder="Choose model" /></div>}
            {activeChatId && <ChatExportMenu chatId={activeChatId} />}
            <span className="connection-dot" /> <span>Local and private</span>
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
    <Link to="/" className="brand" aria-label="Sprinter home" onClick={onNavigate}>
      <span className="brand-mark"><Sparkles size={17} strokeWidth={1.8} /></span>
      <span>SPRINTER</span>
    </Link>

    <div className="sidebar-actions">
      <Link to="/" className="new-chat-button" onClick={onNavigate}>
        <MessageSquarePlus size={16} /><span>New chat</span><kbd>⌘ ⇧ O</kbd>
      </Link>
      <Button className="quiet-button" variant="ghost" onPress={onSearch}><Search size={16} /><span>Search chats</span><kbd>⌘ K</kbd></Button>
    </div>

    <div className="sidebar-section">
      <div className="section-label">YOUR SPACE</div>
      <ConversationList online={online} onNavigate={onNavigate} />
    </div>

    <div className="sidebar-bottom">
      <Link to="/settings" className="sidebar-link" onClick={onNavigate}><Settings2 size={16} /><span>Settings</span></Link>
      <button className="sidebar-link"><CircleHelp size={16} /><span>Help & shortcuts</span></button>
      <div className="sidebar-profile"><div className="profile-avatar">S</div><div><div className="profile-name">Sprinter</div><div className="profile-caption">Private workspace</div></div><Command className="profile-command" size={15} /></div>
      <button className="sidebar-link logout-link" onClick={() => void logout()}><LogOut size={15} /><span>Log out</span></button>
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
  if (query.isLoading) return <div className="sidebar-empty"><span className="empty-dot" />Loading conversations…</div>;
  if (!chats.length) return <div className="sidebar-empty"><span className="empty-dot" />Your conversations will appear here.</div>;
  const groups = new Map<string, typeof chats>();
  for (const chat of chats) {
    const age = Date.now() - chat.updated_at;
    const group = age < 86_400_000 ? "Today" : age < 7 * 86_400_000 ? "Previous 7 days" : "Earlier";
    groups.set(group, [...(groups.get(group) ?? []), chat]);
  }
  return <div className="conversation-list">{["Today", "Previous 7 days", "Earlier"].map((group) => {
    const items = groups.get(group);
    return items?.length ? <div className="conversation-group" key={group}><div className="conversation-group-heading">{group}</div>{items.map((chat) => <div key={chat.id} className="conversation-row"><Link to="/$chatId" params={{ chatId: chat.id }} className="conversation-link" onClick={onNavigate}>{chat.title || "New conversation"}</Link><div className="conversation-actions"><button type="button" aria-label={`Rename ${chat.title || "conversation"}`} disabled={!online} onClick={() => { const title = window.prompt("Rename conversation", chat.title || ""); if (title?.trim()) rename.mutate({ id: chat.id, title: title.trim() }); }}><Pencil size={12} /></button><button type="button" aria-label={`Delete ${chat.title || "conversation"}`} disabled={!online} onClick={() => { if (window.confirm("Delete this conversation?")) remove.mutate(chat.id); }}><Trash2 size={12} /></button></div></div>)}</div> : null;
  })}</div>;
}
