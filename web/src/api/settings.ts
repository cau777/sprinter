import { apiRequest, jsonBody } from "./client";
import type { ApiModel, ModelsResponse, SessionInfo, SettingsResponse, UsageResponse } from "./types.gen";

export type { ApiModel, ModelsResponse };
export type Settings = SettingsResponse;
export type SettingsPatch = Partial<Omit<SettingsResponse, "openrouter_api_key">> & {
  openrouter_api_key?: string | null;
};

export const fetchSettings = () => apiRequest<Settings>("/api/settings");
export const fetchModels = () => apiRequest<ModelsResponse>("/api/models");
export const updateSettings = (patch: SettingsPatch) =>
  apiRequest<Settings>("/api/settings", { method: "PATCH", body: jsonBody(patch) });

export const fetchUsage = () => {
  const tz = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  return apiRequest<UsageResponse>(`/api/usage?tz=${encodeURIComponent(tz)}`);
};
export const fetchSessions = () => apiRequest<SessionInfo[]>("/api/auth/sessions");
export const revokeSession = (id: string) => apiRequest<void>(`/api/auth/sessions/${encodeURIComponent(id)}`, { method: "DELETE" });
export const revokeAllSessions = () => apiRequest<void>("/api/auth/sessions", { method: "DELETE" });
