import { apiRequest, jsonBody } from "./client";
import type { ApiModel, ModelsResponse, SettingsResponse } from "./types.gen";

export type { ApiModel, ModelsResponse };
export type Settings = SettingsResponse;
export type SettingsPatch = Partial<Omit<SettingsResponse, "openrouter_api_key">> & {
  openrouter_api_key?: string | null;
};

export const fetchSettings = () => apiRequest<Settings>("/api/settings");
export const fetchModels = () => apiRequest<ModelsResponse>("/api/models");
export const updateSettings = (patch: SettingsPatch) =>
  apiRequest<Settings>("/api/settings", { method: "PATCH", body: jsonBody(patch) });
