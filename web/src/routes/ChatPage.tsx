import { lazy, Suspense } from "react";

const ChatInterface = lazy(() => import("../chat/ChatInterface").then((module) => ({ default: module.ChatInterface })));

export function ChatPage({ chatId, messageId }: { chatId: string; messageId?: string }) {
  return <Suspense fallback={<div className="m-auto text-[11px] text-[#8c98ad]">Preparing chat…</div>}><ChatInterface chatId={chatId} messageId={messageId} /></Suspense>;
}
