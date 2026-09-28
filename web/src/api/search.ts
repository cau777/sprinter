import { apiRequest } from "./client";
import type { SearchResult } from "./types.gen";

export const searchChats = (query: string, signal?: AbortSignal) =>
  apiRequest<SearchResult[]>(`/api/search?q=${encodeURIComponent(query)}`, { signal });
