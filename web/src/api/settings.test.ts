import { afterEach, describe, expect, it, vi } from "vitest";
import { updateSettingsAndRefetchFavorites } from "./settings";
import type { Settings } from "./settings";

const settings = (favorite_models: string[]) => ({
  favorite_models,
}) as Settings;

afterEach(() => vi.unstubAllGlobals());

describe("updateSettingsAndRefetchFavorites", () => {
  it("refetches settings after changing favorites and returns the server's current favorites", async () => {
    const freshSettings = settings(["anthropic/claude-sonnet-4", "openai/gpt-4.1"]);
    const fetchMock = vi.fn()
      .mockResolvedValueOnce(new Response(JSON.stringify(settings(["openai/gpt-4.1"])), { status: 200 }))
      .mockResolvedValueOnce(new Response(JSON.stringify(freshSettings), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    const result = await updateSettingsAndRefetchFavorites({ favorite_models: freshSettings.favorite_models });

    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(fetchMock.mock.calls[0]?.[0]).toBe("/api/settings");
    expect(fetchMock.mock.calls[0]?.[1]).toMatchObject({ method: "PATCH" });
    expect(JSON.parse(fetchMock.mock.calls[0]?.[1]?.body as string)).toEqual({ favorite_models: freshSettings.favorite_models });
    expect(fetchMock.mock.calls[1]?.[0]).toBe("/api/settings");
    expect(fetchMock.mock.calls[1]?.[1]).toMatchObject({ credentials: "same-origin" });
    expect(fetchMock.mock.calls[1]?.[1]?.method).toBeUndefined();
    expect(result.favorite_models).toEqual(freshSettings.favorite_models);
  });

  it("does not issue an extra fetch for settings changes unrelated to favorites", async () => {
    const savedSettings = settings([]);
    const fetchMock = vi.fn().mockResolvedValueOnce(new Response(JSON.stringify(savedSettings), { status: 200 }));
    vi.stubGlobal("fetch", fetchMock);

    await updateSettingsAndRefetchFavorites({ default_model: "openai/gpt-4.1" });

    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
