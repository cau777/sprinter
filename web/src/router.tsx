import { lazy, Suspense } from "react";
import { createRootRoute, createRoute, createRouter, Outlet, redirect } from "@tanstack/react-router";
import { AppShell } from "./components/AppShell";
import { EmptyChat } from "./routes/EmptyChat";
import { Login } from "./routes/Login";
import { apiRequest } from "./api/client";

const AssistantStoreSpike = lazy(() => import("./routes/AssistantStoreSpike").then((module) => ({ default: module.AssistantStoreSpike })));

const rootRoute = createRootRoute({
  component: () => <Outlet />,
});

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
  component: EmptyChat,
});

const spikeRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/spike/assistant-ui",
  component: () => <Suspense fallback={<div className="route-loading">Opening runtime lab…</div>}><AssistantStoreSpike /></Suspense>,
});

const routeTree = rootRoute.addChildren([loginRoute, appRoute.addChildren([homeRoute, spikeRoute])]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
