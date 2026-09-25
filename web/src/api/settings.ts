import { apiRequest, jsonBody } from "./client";

// Mirrors the corresponding ts-rs DTOs in types.gen.ts; these local aliases will be
// replaced by the generated names when the API structs land with the settings routes.
export type ApiModel = {
  id: string;
  name: string;
  context_length: number;
  pricing: { prompt: string; completion: string };
  input_modalities: string[];
};

export type Settings = {
  openrouter_api_key: { set: boolean; hint: string | null; valid: boolean; readable: boolean };
  default_model: string | null;
  title_model: string | null;
  favorite_models: string[];
  custom_instructions: string;
  pdf_engine: "cloudflare-ai" | "mistral-ocr" | "native";
  upload_limits: Record<string, number>;
};

export type ModelsResponse = { items: ApiModel[] };

export const fetchSettings = () => apiRequest<Settings>("/api/settings");
export const fetchModels = () => apiRequest<ModelsResponse>("/api/models");
export const updateSettings = (patch: Partial<Omit<Settings, "openrouter_api_key">> & { openrouter_api_key?: string | null }) =>
  apiRequest<Settings>("/api/settings", { method: "PATCH", body: jsonBody(patch) });
