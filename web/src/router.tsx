import { createRootRoute, createRoute, createRouter, Outlet } from "@tanstack/react-router";
import { AppShell } from "./components/AppShell";
import { EmptyChat } from "./routes/EmptyChat";
import { AssistantStoreSpike } from "./routes/AssistantStoreSpike";

const rootRoute = createRootRoute({
  component: () => <Outlet />,
});

const appRoute = createRoute({
  getParentRoute: () => rootRoute,
  id: "app",
  component: AppShell,
});

const homeRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/",
  component: EmptyChat,
});

const spikeRoute = createRoute({
  getParentRoute: () => appRoute,
  path: "/spike/assistant-ui",
  component: AssistantStoreSpike,
});

const routeTree = rootRoute.addChildren([appRoute.addChildren([homeRoute, spikeRoute])]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
