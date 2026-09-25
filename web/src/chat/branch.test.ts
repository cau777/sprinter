import { describe, expect, it } from "vitest";
import { siblingsFor, visiblePath } from "./branch";

const messages = [
  { id: "u1", parent_id: null, role: "user", created_at: 1 },
  { id: "a1", parent_id: "u1", role: "assistant", created_at: 2 },
  { id: "u2", parent_id: null, role: "user", created_at: 3 },
  { id: "a2", parent_id: "u2", role: "assistant", created_at: 4 },
  { id: "a3", parent_id: "u2", role: "assistant", created_at: 5 },
];

describe("message branches", () => {
  it("builds only the selected root-to-leaf path", () => {
    expect(visiblePath(messages, "a3").map((message) => message.id)).toEqual(["u2", "a3"]);
    expect(visiblePath(messages, "a1").map((message) => message.id)).toEqual(["u1", "a1"]);
  });

  it("orders siblings by creation time and then ID", () => {
    expect(siblingsFor(messages, messages[4]).map((message) => message.id)).toEqual(["a2", "a3"]);
    expect(siblingsFor(messages, messages[2]).map((message) => message.id)).toEqual(["u1", "u2"]);
  });
});
