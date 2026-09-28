import { describe, expect, it } from "vitest";
import type { ApiModel } from "../api/settings";
import { filterModels, filterModelsByQuickFilters } from "./ModelPicker";

const models: ApiModel[] = [
  { id: "openai/gpt-4.1", name: "GPT-4.1", created_at: null, context_length: 1_000_000, pricing: { prompt: "0", completion: "0" }, input_modalities: ["text"] },
  { id: "anthropic/claude-sonnet-4", name: "Claude Sonnet 4", created_at: null, context_length: 200_000, pricing: { prompt: "0", completion: "0" }, input_modalities: ["text"] },
];

describe("model picker search", () => {
  it("filters OpenRouter models by name or model ID without case sensitivity", () => {
    expect(filterModels(models, "CLAUDE").map((model) => model.id)).toEqual(["anthropic/claude-sonnet-4"]);
    expect(filterModels(models, "GPT-4.1").map((model) => model.id)).toEqual(["openai/gpt-4.1"]);
    expect(filterModels(models, "anthropic/").map((model) => model.id)).toEqual(["anthropic/claude-sonnet-4"]);
  });

  it("returns all models for an empty query and none for an unmatched query", () => {
    expect(filterModels(models, "  ")).toEqual(models);
    expect(filterModels(models, "no such model")).toEqual([]);
  });
});

describe("model picker quick filters", () => {
  const now = Date.UTC(2026, 8, 28);
  const quickFilterModels: ApiModel[] = [
    { id: "new/cheap", name: "New Cheap", created_at: Math.floor(Date.UTC(2026, 5, 1) / 1000), context_length: 100_000, pricing: { prompt: "0.0000004", completion: "0.000001" }, input_modalities: ["text"] },
    { id: "new/expensive", name: "New Expensive", created_at: Date.UTC(2026, 6, 1), context_length: 100_000, pricing: { prompt: "0.0000006", completion: "0.000001" }, input_modalities: ["text"] },
    { id: "new/limit", name: "At Price Limit", created_at: Math.floor(Date.UTC(2026, 6, 1) / 1000), context_length: 100_000, pricing: { prompt: "0.0000005", completion: "0.000001" }, input_modalities: ["text"] },
    { id: "old/cheap", name: "Old Cheap", created_at: Math.floor(Date.UTC(2026, 2, 27) / 1000), context_length: 100_000, pricing: { prompt: "0.0000001", completion: "0.000001" }, input_modalities: ["text"] },
    { id: "unknown/cheap", name: "Unknown Cheap", created_at: null, context_length: 100_000, pricing: { prompt: "0.0000001", completion: "0.000001" }, input_modalities: ["text"] },
  ];

  it("shows models from the past six months by default and can turn that filter off", () => {
    expect(filterModelsByQuickFilters(quickFilterModels, { newModelsOnly: true, cheapOnly: false }, now).map((model) => model.id))
      .toEqual(["new/cheap", "new/expensive", "new/limit"]);
    expect(filterModelsByQuickFilters(quickFilterModels, { newModelsOnly: false, cheapOnly: false }, now))
      .toHaveLength(quickFilterModels.length);
  });

  it("filters input prices below $0.50 per million and combines both filters", () => {
    expect(filterModelsByQuickFilters(quickFilterModels, { newModelsOnly: false, cheapOnly: true }, now).map((model) => model.id))
      .toEqual(["new/cheap", "old/cheap", "unknown/cheap"]);
    expect(filterModelsByQuickFilters(quickFilterModels, { newModelsOnly: true, cheapOnly: true }, now).map((model) => model.id))
      .toEqual(["new/cheap"]);
  });
});
