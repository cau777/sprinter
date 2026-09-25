import { Link, Outlet } from "@tanstack/react-router";
import { Button } from "@heroui/react";
import { Bot, CircleHelp, Command, MessageSquarePlus, Search, Settings2, Sparkles } from "lucide-react";

export function AppShell() {
  return (
    <main className="app-frame">
      <aside className="sidebar glass-panel">
        <Link to="/" className="brand" aria-label="Sprinter home">
          <span className="brand-mark"><Sparkles size={17} strokeWidth={1.8} /></span>
          <span>SPRINTER</span>
        </Link>

        <div className="sidebar-actions">
          <Button className="new-chat-button" variant="outline" onPress={() => window.location.assign("/")}>
            <MessageSquarePlus size={16} /> <span>New chat</span>
            <kbd>⌘ K</kbd>
          </Button>
          <Button className="quiet-button" variant="ghost"><Search size={16} /><span>Search chats</span></Button>
        </div>

        <div className="sidebar-section">
          <div className="section-label">YOUR SPACE</div>
          <div className="sidebar-empty"><span className="empty-dot" />Your conversations will appear here.</div>
        </div>

        <div className="sidebar-bottom">
          <Link to="/spike/assistant-ui" className="sidebar-link"><Bot size={16} /><span>Runtime spike</span><span className="demo-tag">DEMO</span></Link>
          <button className="sidebar-link"><Settings2 size={16} /><span>Settings</span></button>
          <button className="sidebar-link"><CircleHelp size={16} /><span>Help & shortcuts</span></button>
          <div className="sidebar-profile"><div className="profile-avatar">S</div><div><div className="profile-name">Sprinter</div><div className="profile-caption">Private workspace</div></div><Command className="profile-command" size={15} /></div>
        </div>
      </aside>

      <section className="main-panel">
        <header className="topbar">
          <div className="breadcrumb"><span className="topbar-kicker">WORKSPACE</span><span className="breadcrumb-slash">/</span><span className="breadcrumb-current">New conversation</span></div>
          <div className="topbar-right"><span className="connection-dot" /> <span>Local and private</span></div>
        </header>
        <Outlet />
      </section>
    </main>
  );
}
