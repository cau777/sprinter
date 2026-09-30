import { describe, expect, it } from "vitest";
import { setToolSelection } from "./toolSelection";

describe("setToolSelection", () => {
  it("removes the selected tool and replaces incompatible selections", () => {
    const withBash = setToolSelection([], "bash", true);
    expect(withBash).toEqual(["bash"]);
    const withSearch = setToolSelection(withBash, "web_search", true);
    expect(withSearch).toEqual(["web_search"]);
    expect(setToolSelection(withSearch, "web_search", false)).toEqual([]);
  });
});
