import { lazy, Suspense } from "react";
import { createRootRoute, createRoute, createRouter, Outlet, redirect } from "@tanstack/react-router";
import { AppShell } from "./components/AppShell";
import { EmptyChat } from "./routes/EmptyChat";
import { Login } from "./routes/Login";
import { apiRequest } from "./api/client";
import { fetchSettings } from "./api/settings";

const Setup = lazy(() => import("./routes/Setup").then((module) => ({ default: module.Setup })));
const SettingsPage = lazy(() => import("./routes/SettingsPage").then((module) => ({ default: module.SettingsPage })));
const ChatPage = lazy(() => import("./routes/ChatPage").then((module) => ({ default: module.ChatPage })));

const rootRoute = createRootRoute({
  component: () => <Outlet />,
});

const AssistantStoreSpike = lazy(() => import("./routes/AssistantStoreSpike").then((module) => ({ default: module.AssistantStoreSpike })));

function isOfflineFailure(error: unknown) {
  return !navigator.onLine || error instanceof TypeError;
}

const appRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "app",
  beforeLoad: async ({ location }) => {
    try {
      await apiRequest("/api/auth/sessions", {}, { redirectOnUnauthorized: false });
    } catch (error) {
      if (error instanceof Error && "status" in error && error.status === 401) {
        throw redirect({ to: "/login", search: { redirect: `${location.pathname}${location.searchStr}` } });
      }
      if (isOfflineFailure(error)) return;
      throw error;
    }
  },
  component: AppShell,
});

const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  validateSearch: (search: Record<string, unknown>) => ({
    redirect: typeof search.redirect === "string" ? search.redirect : undefined,
  }),
  component: Login,
});

const homeRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/",
  beforeLoad: async () => {
    let settings: Awaited<ReturnType<typeof fetchSettings>>;
    try {
      settings = await fetchSettings();
    } catch (error) {
      if (isOfflineFailure(error)) return;
      throw error;
    }
    if (!settings.openrouter_api_key.set || !settings.default_model) throw redirect({ to: "/setup" });
  },
  component: EmptyChat,
});

const setupRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/setup",
  component: () => <Suspense fallback={<div className="m-auto text-[11px] text-[#8c98ad]">Preparing your workspace…</div>}><Setup /></Suspense>,
});

const settingsRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/settings",
  component: () => <Suspense fallback={<div className="m-auto text-[11px] text-[#8c98ad]">Loading settings…</div>}><SettingsPage /></Suspense>,
});

const spikeRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/spike/assistant-ui",
  component: () => <Suspense fallback={<div className="m-auto text-[11px] text-[#8c98ad]">Opening runtime lab…</div>}><AssistantStoreSpike /></Suspense>,
});

const chatRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/$chatId",
  validateSearch: (search: Record<string, unknown>) => ({
    messageId: typeof search.messageId === "string" ? search.messageId : undefined,
  }),
  component: function ChatRoute() {
    const { chatId } = chatRoute.useParams();
    const { messageId } = chatRoute.useSearch();
    return <Suspense fallback={<div className="m-auto text-[11px] text-[#8c98ad]">Opening conversation…</div>}><ChatPage chatId={chatId} messageId={messageId} /></Suspense>;
  },
});

const routeTree = rootRoute.addChildren([loginRoute, appRoute.addChildren([homeRoute, setupRoute, settingsRoute, spikeRoute, chatRoute])]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
