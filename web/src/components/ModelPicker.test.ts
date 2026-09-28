import { describe, expect, it } from "vitest";
import type { ApiModel } from "../api/settings";
import { filterModels } from "./ModelPicker";

const models: ApiModel[] = [
  { id: "openai/gpt-4.1", name: "GPT-4.1", context_length: 1_000_000, pricing: { prompt: "0", completion: "0" }, input_modalities: ["text"] },
  { id: "anthropic/claude-sonnet-4", name: "Claude Sonnet 4", context_length: 200_000, pricing: { prompt: "0", completion: "0" }, input_modalities: ["text"] },
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
