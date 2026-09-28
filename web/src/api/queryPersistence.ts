import { del, get, set } from "idb-keyval";
import type { Persister, PersistedClient } from "@tanstack/react-query-persist-client";

const persistedQueryKey = "sprinter-query-cache-v1";
let persistenceEnabled = true;

export const queryPersister: Persister = {
  persistClient: (client: PersistedClient) => persistenceEnabled ? set(persistedQueryKey, client) : undefined,
  restoreClient: () => get<PersistedClient>(persistedQueryKey),
  removeClient: () => del(persistedQueryKey),
};

export async function clearPersistedQueryCache() {
  persistenceEnabled = false;
  await queryPersister.removeClient();
}

export function enableQueryPersistence() {
  persistenceEnabled = true;
}
