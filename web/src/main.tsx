import "@fontsource/sora/500.css";
import "@fontsource/sora/700.css";
import "@fontsource/manrope/400.css";
import "@fontsource/manrope/600.css";
import "@fontsource/jetbrains-mono/400.css";
import "./theme/app.css";

import React from "react";
import ReactDOM from "react-dom/client";
import { PersistQueryClientProvider } from "@tanstack/react-query-persist-client";
import { RouterProvider } from "@tanstack/react-router";
import { router } from "./router";
import { ClientErrorBoundary, installGlobalErrorHandlers } from "./clientErrors";
import { queryPersister } from "./api/queryPersistence";
import { queryClient } from "./api/queryClient";

installGlobalErrorHandlers();

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ClientErrorBoundary>
      <PersistQueryClientProvider
        client={queryClient}
        persistOptions={{
          persister: queryPersister,
          maxAge: 7 * 24 * 60 * 60 * 1000,
          buster: "sprinter-v1",
          dehydrateOptions: {
            shouldDehydrateQuery: (query) =>
              query.state.status === "success" &&
              (query.queryKey[0] === "chats" || query.queryKey[0] === "chat"),
          },
        }}
      >
        <RouterProvider router={router} />
      </PersistQueryClientProvider>
    </ClientErrorBoundary>
  </React.StrictMode>,
);
