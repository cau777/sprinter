import { del, get, set } from "idb-keyval";
import type { Persister, PersistedClient } from "@tanstack/react-query-persist-client";

const persistedQueryKey = "sprinter-query-cache-v1";

export const queryPersister: Persister = {
  persistClient: (client: PersistedClient) => set(persistedQueryKey, client),
  restoreClient: () => get<PersistedClient>(persistedQueryKey),
  removeClient: () => del(persistedQueryKey),
};

export async function clearPersistedQueryCache() {
  await queryPersister.removeClient();
}
