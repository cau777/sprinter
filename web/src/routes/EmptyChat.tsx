import { lazy, Suspense } from "react";

const ChatInterface = lazy(() => import("../chat/ChatInterface").then((module) => ({ default: module.ChatInterface })));

export function EmptyChat() {
  return <Suspense fallback={<div className="route-loading">Preparing chat…</div>}><ChatInterface /></Suspense>;
}
