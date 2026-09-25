import { Button } from "@heroui/react";
import { ArrowUpRight, CornerDownLeft, Paperclip, Sparkles } from "lucide-react";
import { DefaultModelPicker } from "../components/DefaultModelPicker";

export function EmptyChat() {
  return (
    <section className="conversation-stage">
      <div className="welcome-content">
        <div className="welcome-icon"><Sparkles size={22} /></div>
        <p className="eyebrow">A CLEARER WAY TO THINK</p>
        <h1>What’s on your mind<span>?</span></h1>
        <p className="welcome-copy">A thought, a question, a half-formed idea.<br />Start anywhere. We’ll take it from there.</p>
        <div className="prompt-suggestions">
          <button className="suggestion-card"><span className="suggestion-icon violet">✳</span><span><b>Think it through</b><small>Help me explore an idea</small></span><ArrowUpRight size={15} /></button>
          <button className="suggestion-card"><span className="suggestion-icon blue">⌘</span><span><b>Make something</b><small>Write, plan, or create</small></span><ArrowUpRight size={15} /></button>
          <button className="suggestion-card"><span className="suggestion-icon gold">◒</span><span><b>Get unstuck</b><small>Break down a problem</small></span><ArrowUpRight size={15} /></button>
        </div>
      </div>

      <div className="composer-wrap">
        <form className="composer-card" onSubmit={(event) => event.preventDefault()}>
          <textarea aria-label="Message" placeholder="Message Sprinter…" rows={1} />
          <div className="composer-toolbar">
            <div className="composer-left"><Button isIconOnly className="attach-button" variant="ghost" aria-label="Attach file"><Paperclip size={17} /></Button><DefaultModelPicker /></div>
            <div className="composer-right"><span className="enter-hint"><CornerDownLeft size={12} /> to send</span><Button isIconOnly className="send-button" aria-label="Send message"><ArrowUpRight size={17} /></Button></div>
          </div>
        </form>
        <p className="composer-caption">Sprinter can make mistakes. Check important information.</p>
      </div>
    </section>
  );
}
