import { useState } from "react";
import { Link, Outlet } from "@tanstack/react-router";
import { Button } from "@heroui/react";
import { Bot, CircleHelp, Command, LogOut, Menu, MessageSquarePlus, Search, Settings2, Sparkles, X } from "lucide-react";
import { apiRequest } from "../api/client";

export function AppShell() {
  const [drawerOpen, setDrawerOpen] = useState(false);

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
            <span className="topbar-kicker">WORKSPACE</span><span className="breadcrumb-slash">/</span><span className="breadcrumb-current">New conversation</span>
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
      <div className="sidebar-empty"><span className="empty-dot" />Your conversations will appear here.</div>
    </div>

    <div className="sidebar-bottom">
      <Link to="/spike/assistant-ui" className="sidebar-link" onClick={onNavigate}><Bot size={16} /><span>Runtime spike</span><span className="demo-tag">DEMO</span></Link>
      <button className="sidebar-link"><Settings2 size={16} /><span>Settings</span></button>
      <button className="sidebar-link"><CircleHelp size={16} /><span>Help & shortcuts</span></button>
      <div className="sidebar-profile"><div className="profile-avatar">S</div><div><div className="profile-name">Sprinter</div><div className="profile-caption">Private workspace</div></div><Command className="profile-command" size={15} /></div>
      <button className="sidebar-link logout-link" onClick={() => void logout()}><LogOut size={15} /><span>Log out</span></button>
    </div>
  </>;
}
