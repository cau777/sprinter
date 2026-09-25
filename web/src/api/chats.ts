import { apiRequest, jsonBody } from "./client";

export type ChatSummary = { id: string; title: string | null; model: string; updated_at: number };
export type ChatPage = { items: ChatSummary[]; next_cursor: string | null };
export type ChatAttachment = { upload_id: string; position: number; pdf_engine: string | null; parse_cache: string | null };
export type ChatMessage = {
  id: string;
  chat_id: string;
  parent_id: string | null;
  role: "user" | "assistant";
  content: string;
  status: "streaming" | "complete" | "cancelled" | "error" | "interrupted";
  error: string | null;
  model: string | null;
  generation_id: string | null;
  finish_reason: string | null;
  prompt_tokens: number | null;
  completion_tokens: number | null;
  reasoning_tokens: number | null;
  cost: number | null;
  created_at: number;
  updated_at: number;
  attachments: ChatAttachment[];
};
export type ChatDetail = {
  id: string;
  title: string | null;
  title_source: string;
  model: string;
  current_leaf_id: string | null;
  created_at: number;
  updated_at: number;
  messages: ChatMessage[];
};
export type SendMessageResponse = { chat?: ChatSummary; user_message: ChatMessage | null; assistant_message: ChatMessage };
export type StreamEvent =
  | { event: "snapshot" | "delta"; content: string }
  | { event: "done"; status: ChatMessage["status"]; finish_reason?: string | null; cost?: number | null }
  | { event: "error"; status?: "error"; message: string }
  | { event: "title"; chat_id: string; title: string };

export const fetchChats = () => apiRequest<ChatPage>("/api/chats");
export const fetchChat = (id: string) => apiRequest<ChatDetail>(`/api/chats/${encodeURIComponent(id)}`);
export const updateChat = (id: string, patch: { title?: string; model?: string }) =>
  apiRequest<ChatSummary>(`/api/chats/${encodeURIComponent(id)}`, { method: "PATCH", body: jsonBody(patch) });
export const deleteChat = (id: string) => apiRequest<void>(`/api/chats/${encodeURIComponent(id)}`, { method: "DELETE" });
export const sendToNewChat = (content: string, model?: string) => apiRequest<SendMessageResponse>("/api/chats/new/messages", {
  method: "POST",
  body: jsonBody({ parent_id: null, content, attachment_ids: [], ...(model ? { model } : {}) }),
});
export const sendToChat = (id: string, parent_id: string | null, content: string, model?: string) => apiRequest<SendMessageResponse>(`/api/chats/${encodeURIComponent(id)}/messages`, {
  method: "POST",
  body: jsonBody({ parent_id, content, attachment_ids: [], ...(model ? { model } : {}) }),
});
export const regenerateMessage = (id: string) => apiRequest<{ assistant_message: ChatMessage }>(`/api/messages/${encodeURIComponent(id)}/regenerate`, { method: "POST", body: jsonBody({}) });
export const cancelMessage = (id: string) => apiRequest<void>(`/api/messages/${encodeURIComponent(id)}/cancel`, { method: "POST", body: "{}" });

const delay = (ms: number) => new Promise((resolve) => window.setTimeout(resolve, ms));

/** Attach to the server-owned generation. A failed connection retries from a fresh snapshot. */
export async function watchMessage(messageId: string, onEvent: (event: StreamEvent) => void, signal: AbortSignal) {
  let retry = 0;
  while (!signal.aborted) {
    try {
      const response = await fetch(`/api/messages/${encodeURIComponent(messageId)}/stream`, { credentials: "same-origin", signal });
      if (!response.ok || !response.body) throw new Error(`Stream request failed (${response.status})`);
      retry = 0;
      const reader = response.body.getReader();
      const decoder = new TextDecoder();
      let buffer = "";
      let eventName = "message";
      let data: string[] = [];
      let terminal = false;
      const dispatch = () => {
        if (!data.length) { eventName = "message"; return; }
        try {
          onEvent({ event: eventName, ...JSON.parse(data.join("\n")) } as StreamEvent);
          if (eventName === "done" || eventName === "error") terminal = true;
        }
        catch { /* Ignore malformed provider events and keep the stream alive. */ }
        data = [];
        eventName = "message";
      };
      while (!signal.aborted) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        let boundary: number;
        while ((boundary = buffer.indexOf("\n")) >= 0) {
          const line = buffer.slice(0, boundary).replace(/\r$/, "");
          buffer = buffer.slice(boundary + 1);
          if (line === "") dispatch();
          else if (line.startsWith("event:")) eventName = line.slice(6).trim();
          else if (line.startsWith("data:")) data.push(line.slice(5).trimStart());
        }
      }
      await reader.cancel().catch(() => undefined);
      if (signal.aborted || terminal) return;
    } catch (error) {
      if (signal.aborted || (error instanceof DOMException && error.name === "AbortError")) return;
    }
    retry += 1;
    await delay(Math.min(5000, 400 * 2 ** Math.min(retry, 4)));
  }
}
