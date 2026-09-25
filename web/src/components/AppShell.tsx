import { useState } from "react";
import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import { Button } from "@heroui/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Bot, CircleHelp, Command, LogOut, Menu, MessageSquarePlus, Pencil, Search, Settings2, Sparkles, Trash2, X } from "lucide-react";
import { apiRequest } from "../api/client";
import { deleteChat, fetchChats, updateChat } from "../api/chats";

export function AppShell() {
  const [drawerOpen, setDrawerOpen] = useState(false);
  const currentPath = useRouterState({ select: (state) => state.location.pathname });
  const chatList = useQuery({ queryKey: ["chats"], queryFn: fetchChats });
  const currentChat = chatList.data?.items.find((chat) => `/${chat.id}` === currentPath);
  const pageName = currentPath === "/settings" ? "Settings" : currentPath === "/setup" ? "Workspace setup" : currentPath === "/spike/assistant-ui" ? "Runtime lab" : currentChat?.title || (currentChat ? "Conversation" : currentPath === "/" ? "New conversation" : "Conversation");

  return (
    <main className="app-frame">
      <aside className="sidebar glass-panel desktop-sidebar">
        <SidebarContents onNavigate={() => setDrawerOpen(false)} />
      </aside>

      {drawerOpen && <div className="mobile-drawer-scrim" onClick={() => setDrawerOpen(false)}>
        <aside className="sidebar glass-panel mobile-sidebar" onClick={(event) => event.stopPropagation()}>
          <Button isIconOnly className="drawer-close" variant="ghost" aria-label="Close navigation" onPress={() => setDrawerOpen(false)}><X size={17} /></Button>
          <SidebarContents onNavigate={() => setDrawerOpen(false)} />
        </aside>
      </div>}

      <section className="main-panel">
        <header className="topbar">
          <div className="breadcrumb">
            <Button isIconOnly className="mobile-menu-button" variant="ghost" aria-label="Open navigation" onPress={() => setDrawerOpen(true)}><Menu size={17} /></Button>
            <span className="topbar-kicker">WORKSPACE</span><span className="breadcrumb-slash">/</span><span className="breadcrumb-current">{pageName}</span>
          </div>
          <div className="topbar-right"><span className="connection-dot" /> <span>Local and private</span></div>
        </header>
        <Outlet />
      </section>
    </main>
  );
}

function SidebarContents({ onNavigate }: { onNavigate: () => void }) {
  async function logout() {
    await apiRequest("/api/auth/logout", { method: "POST", body: "{}" }).catch(() => undefined);
    window.location.assign("/login");
  }

  return <>
    <Link to="/" className="brand" aria-label="Sprinter home" onClick={onNavigate}>
      <span className="brand-mark"><Sparkles size={17} strokeWidth={1.8} /></span>
      <span>SPRINTER</span>
    </Link>

    <div className="sidebar-actions">
      <Link to="/" className="new-chat-button" onClick={onNavigate}>
        <MessageSquarePlus size={16} /><span>New chat</span><kbd>⌘ K</kbd>
      </Link>
      <Button className="quiet-button" variant="ghost"><Search size={16} /><span>Search chats</span></Button>
    </div>

    <div className="sidebar-section">
      <div className="section-label">YOUR SPACE</div>
      <ConversationList onNavigate={onNavigate} />
    </div>

    <div className="sidebar-bottom">
      <Link to="/spike/assistant-ui" className="sidebar-link" onClick={onNavigate}><Bot size={16} /><span>Runtime spike</span><span className="demo-tag">DEMO</span></Link>
      <Link to="/settings" className="sidebar-link" onClick={onNavigate}><Settings2 size={16} /><span>Settings</span></Link>
      <button className="sidebar-link"><CircleHelp size={16} /><span>Help & shortcuts</span></button>
      <div className="sidebar-profile"><div className="profile-avatar">S</div><div><div className="profile-name">Sprinter</div><div className="profile-caption">Private workspace</div></div><Command className="profile-command" size={15} /></div>
      <button className="sidebar-link logout-link" onClick={() => void logout()}><LogOut size={15} /><span>Log out</span></button>
    </div>
  </>;
}

function ConversationList({ onNavigate }: { onNavigate: () => void }) {
  const query = useQuery({ queryKey: ["chats"], queryFn: fetchChats });
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const rename = useMutation({ mutationFn: ({ id, title }: { id: string; title: string }) => updateChat(id, { title }), onSuccess: () => queryClient.invalidateQueries({ queryKey: ["chats"] }) });
  const remove = useMutation({ mutationFn: (id: string) => deleteChat(id), onSuccess: (_, id) => { queryClient.invalidateQueries({ queryKey: ["chats"] }); queryClient.removeQueries({ queryKey: ["chat", id] }); if (window.location.pathname === `/${id}`) void navigate({ to: "/" }); } });
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
    return items?.length ? <div className="conversation-group" key={group}><div className="conversation-group-heading">{group}</div>{items.map((chat) => <div key={chat.id} className="conversation-row"><Link to="/$chatId" params={{ chatId: chat.id }} className="conversation-link" onClick={onNavigate}>{chat.title || "New conversation"}</Link><div className="conversation-actions"><button type="button" aria-label={`Rename ${chat.title || "conversation"}`} onClick={() => { const title = window.prompt("Rename conversation", chat.title || ""); if (title?.trim()) rename.mutate({ id: chat.id, title: title.trim() }); }}><Pencil size={12} /></button><button type="button" aria-label={`Delete ${chat.title || "conversation"}`} onClick={() => { if (window.confirm("Delete this conversation?")) remove.mutate(chat.id); }}><Trash2 size={12} /></button></div></div>)}</div> : null;
  })}</div>;
}
