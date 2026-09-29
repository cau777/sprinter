import { apiRequest, jsonBody } from "./client";
import type {
  ChatDetail as GeneratedChatDetail,
  ChatPage,
  ChatSummary,
  MessageRecord,
  NewChatMessageResponse,
  RegenerateMessageResponse,
  SendMessageResponse,
  SwitchBranchResponse,
} from "./types.gen";

export type { ChatPage, ChatSummary };
export type ChatAttachment = MessageRecord["attachments"][number];
export type ChatMessage = Omit<MessageRecord, "role" | "status"> & {
  role: "user" | "assistant";
  status: "streaming" | "complete" | "cancelled" | "error" | "interrupted";
};
export type ChatDetail = Omit<GeneratedChatDetail, "messages"> & { messages: ChatMessage[] };
export type NewChatResponse = Omit<NewChatMessageResponse, "user_message" | "assistant_message"> & { user_message: ChatMessage; assistant_message: ChatMessage };
export type ExistingChatResponse = Omit<SendMessageResponse, "user_message" | "assistant_message"> & { user_message: ChatMessage; assistant_message: ChatMessage };
export type RetryResponse = Omit<RegenerateMessageResponse, "assistant_message"> & { assistant_message: ChatMessage };
export type StreamEvent =
  | { event: "snapshot" | "delta"; content: string }
  | { event: "done"; status: ChatMessage["status"]; finish_reason?: string | null; usage?: { prompt_tokens?: number | null; completion_tokens?: number | null; reasoning_tokens?: number | null } | null; cost?: number | null }
  | { event: "error"; status?: "error"; message: string }
  | { event: "title"; chat_id: string; title: string };

export const fetchChats = () => apiRequest<ChatPage>("/api/chats");
const asMessage = (message: MessageRecord): ChatMessage => ({
  ...message,
  role: message.role as ChatMessage["role"],
  status: message.status as ChatMessage["status"],
});
export const fetchChat = async (id: string): Promise<ChatDetail> => {
  const chat = await apiRequest<GeneratedChatDetail>(`/api/chats/${encodeURIComponent(id)}`);
  return { ...chat, messages: chat.messages.map(asMessage) };
};
export const updateChat = (id: string, patch: { title?: string; model?: string }) =>
  apiRequest<ChatSummary>(`/api/chats/${encodeURIComponent(id)}`, { method: "PATCH", body: jsonBody(patch) });
export const deleteChat = (id: string) => apiRequest<void>(`/api/chats/${encodeURIComponent(id)}`, { method: "DELETE" });
export const sendToNewChat = async (content: string, model?: string, attachmentIds: string[] = []): Promise<NewChatResponse> => {
  const result = await apiRequest<NewChatMessageResponse>("/api/chats/new/messages", {
  method: "POST",
  body: jsonBody({ parent_id: null, content, attachment_ids: attachmentIds, ...(model ? { model } : {}) }),
  });
  return { ...result, user_message: asMessage(result.user_message), assistant_message: asMessage(result.assistant_message) };
};
export const sendToChat = async (id: string, parent_id: string | null, content: string, model?: string, attachmentIds: string[] = []): Promise<ExistingChatResponse> => {
  const result = await apiRequest<SendMessageResponse>(`/api/chats/${encodeURIComponent(id)}/messages`, {
  method: "POST",
  body: jsonBody({ parent_id, content, attachment_ids: attachmentIds, ...(model ? { model } : {}) }),
  });
  return { ...result, user_message: asMessage(result.user_message), assistant_message: asMessage(result.assistant_message) };
};
export const regenerateMessage = async (id: string, model?: string): Promise<RetryResponse> => {
  const result = await apiRequest<RegenerateMessageResponse>(`/api/messages/${encodeURIComponent(id)}/regenerate`, { method: "POST", body: jsonBody(model ? { model } : {}) });
  return { assistant_message: asMessage(result.assistant_message) };
};
export const switchBranch = (chatId: string, messageId: string) =>
  apiRequest<SwitchBranchResponse>(`/api/chats/${encodeURIComponent(chatId)}/switch`, { method: "POST", body: jsonBody({ message_id: messageId }) });
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
