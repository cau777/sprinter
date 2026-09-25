import { useEffect, useMemo, useState } from "react";
import type { AppendMessage, ThreadMessageLike } from "@assistant-ui/react";
import {
  ActionBarPrimitive,
  AssistantRuntimeProvider,
  ComposerPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  useExternalStoreRuntime,
} from "@assistant-ui/react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowUpRight, RotateCw, Sparkles, Square } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { cancelMessage, fetchChat, regenerateMessage, sendToChat, sendToNewChat, switchBranch, updateChat, watchMessage } from "../api/chats";
import type { ChatDetail, ChatMessage, ChatSummary } from "../api/chats";
import { ApiError } from "../api/client";
import { fetchModels, fetchSettings } from "../api/settings";
import { DefaultModelPicker } from "../components/DefaultModelPicker";
import { ModelPicker } from "../components/ModelPicker";
import { siblingsFor, visiblePath } from "./branch";

type Props = { chatId?: string };
type RuntimeMessage = ThreadMessageLike & { id: string; parentId: string | null; generationStatus: ChatMessage["status"]; error?: string | null; model?: string | null };
type SendResult = { chatId: string; chat?: ChatSummary; user_message: ChatMessage; assistant_message: ChatMessage };

function textOf(message: AppendMessage) {
  return message.content.filter((part) => part.type === "text").map((part) => part.text).join("");
}

function applyMessageEvent(queryClient: ReturnType<typeof useQueryClient>, chatId: string, messageId: string, event: { event: string; [key: string]: unknown }) {
  if (event.event === "title") {
    queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => current ? { ...current, title: String(event.title) } : current);
    void queryClient.invalidateQueries({ queryKey: ["chats"] });
    return;
  }
  queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => {
    if (!current) return current;
    return { ...current, messages: current.messages.map((message) => {
      if (message.id !== messageId) return message;
      if (event.event === "snapshot") return { ...message, content: String(event.content ?? "") };
      if (event.event === "delta") return { ...message, content: message.content + String(event.content ?? "") };
      if (event.event === "done") return { ...message, status: String(event.status ?? "complete") as ChatMessage["status"], finish_reason: (event.finish_reason as string | null) ?? null, cost: (event.cost as number | null) ?? message.cost };
      if (event.event === "error") return { ...message, status: "error", error: String(event.message ?? "Generation failed") };
      return message;
    }) };
  });
}

export function ChatInterface({ chatId }: Props) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const chatQuery = useQuery({ queryKey: ["chat", chatId], queryFn: () => fetchChat(chatId!), enabled: Boolean(chatId) });
  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: fetchSettings });
  const modelQuery = useQuery({ queryKey: ["models"], queryFn: fetchModels, enabled: Boolean(chatId) });
  const [coarsePointer, setCoarsePointer] = useState(false);
  const chatModelMutation = useMutation({ mutationFn: (model: string) => updateChat(chatId!, { model }), onSuccess: (summary) => {
    queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => current ? { ...current, model: summary.model } : current);
    void queryClient.invalidateQueries({ queryKey: ["chats"] });
  } });
  const [draftError, setDraftError] = useState<string>();
  const [retryModels, setRetryModels] = useState<Record<string, string>>({});
  const detail = chatQuery.data;

  useEffect(() => {
    const media = window.matchMedia("(pointer: coarse)");
    const update = () => setCoarsePointer(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  const visibleMessages = useMemo(() => visiblePath(detail?.messages ?? [], detail?.current_leaf_id ?? null), [detail]);
  const streamingMessage = visibleMessages.find((message) => message.role === "assistant" && message.status === "streaming");
  const latestAssistant = visibleMessages.at(-1)?.role === "assistant" ? visibleMessages.at(-1) : undefined;

  useEffect(() => {
    if (!chatId || !latestAssistant) return;
    const controller = new AbortController();
    void watchMessage(latestAssistant.id, (event) => applyMessageEvent(queryClient, chatId, latestAssistant.id, event), controller.signal);
    return () => controller.abort();
  }, [chatId, queryClient, latestAssistant?.id]);

  const send = useMutation({
    mutationFn: async ({ content, parentId }: { content: string; parentId: string | null }): Promise<SendResult> => {
      if (chatId) {
        const response = await sendToChat(chatId, parentId, content);
        return { chatId, user_message: response.user_message, assistant_message: response.assistant_message };
      }
      const response = await sendToNewChat(content);
      return { chatId: response.chat.id, chat: response.chat, user_message: response.user_message, assistant_message: response.assistant_message };
    },
    onSuccess: ({ chatId: nextChatId, chat, user_message, assistant_message }) => {
      setDraftError(undefined);
      queryClient.setQueryData<ChatDetail>(["chat", nextChatId], (current) => {
        if (current) return { ...current, current_leaf_id: assistant_message.id, messages: [...current.messages, user_message, assistant_message] };
        const now = Date.now();
        return {
          id: nextChatId, title: chat?.title ?? null, title_source: "auto", model: chat?.model ?? settingsQuery.data?.default_model ?? "", current_leaf_id: assistant_message.id, created_at: now, updated_at: now,
          messages: [user_message, assistant_message],
        };
      });
      void queryClient.invalidateQueries({ queryKey: ["chats"] });
      if (!chatId) void navigate({ to: "/$chatId", params: { chatId: nextChatId } });
    },
    onError: (error) => setDraftError(error instanceof ApiError ? error.message : "Could not send this message."),
  });

  const retry = useMutation({
    mutationFn: ({ messageId, model }: { messageId: string; model?: string }) => regenerateMessage(messageId, model),
    onSuccess: ({ assistant_message }) => {
      if (!chatId) return;
      queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => current ? { ...current, current_leaf_id: assistant_message.id, messages: [...current.messages, assistant_message] } : current);
    },
    onError: (error) => setDraftError(error instanceof ApiError ? error.message : "Could not retry this response."),
  });
  const switchMutation = useMutation({
    mutationFn: (messageId: string) => switchBranch(chatId!, messageId),
    onSuccess: ({ current_leaf_id }) => {
      queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => current ? { ...current, current_leaf_id } : current);
    },
    onError: (error) => setDraftError(error instanceof ApiError ? error.message : "Could not switch branches."),
  });

  const messageList: RuntimeMessage[] = visibleMessages.map((message) => ({
    id: message.id,
    parentId: message.parent_id,
    role: message.role,
    content: message.content,
    generationStatus: message.status,
    error: message.error,
    model: message.model,
  }));

  const runtime = useExternalStoreRuntime({
    messages: messageList,
    isRunning: Boolean(streamingMessage),
    convertMessage: (message) => ({ ...message, content: message.content }),
    setMessages: () => undefined,
    onNew: async (message) => {
      const content = textOf(message).trim();
      if (content) send.mutate({ content, parentId: message.parentId ?? detail?.current_leaf_id ?? null });
    },
    onEdit: async (message) => {
      const content = textOf(message).trim();
      if (content) send.mutate({ content, parentId: message.parentId ?? null });
    },
    onReload: async (parentId) => {
      const original = visibleMessages.find((message) => message.role === "assistant" && message.parent_id === parentId);
      if (original) retry.mutate({ messageId: original.id });
    },
    onCancel: async () => { if (streamingMessage) await cancelMessage(streamingMessage.id).catch(() => undefined); },
  });

  if (chatId && chatQuery.isLoading) return <section className="chat-loading">Opening conversation…</section>;
  if (chatId && chatQuery.isError) return <section className="chat-loading" role="alert">{chatQuery.error.message}</section>;
  const hasMessages = visibleMessages.length > 0;

  return <section className={`conversation-stage ${hasMessages ? "has-messages" : ""}`}>
    {!hasMessages && <div className="welcome-content">
      <div className="welcome-icon"><Sparkles size={22} /></div>
      <p className="eyebrow">A CLEARER WAY TO THINK</p>
      <h1>What’s on your mind<span>?</span></h1>
      <p className="welcome-copy">A thought, a question, a half-formed idea.<br />Start anywhere. We’ll take it from there.</p>
      <div className="prompt-suggestions">
        <button className="suggestion-card" type="button" onClick={() => send.mutate({ content: "Help me think through an idea.", parentId: null })}><span className="suggestion-icon violet">✳</span><span><b>Think it through</b><small>Help me explore an idea</small></span></button>
        <button className="suggestion-card" type="button" onClick={() => send.mutate({ content: "Help me make something.", parentId: null })}><span className="suggestion-icon blue">⌘</span><span><b>Make something</b><small>Write, plan, or create</small></span></button>
        <button className="suggestion-card" type="button" onClick={() => send.mutate({ content: "Help me get unstuck.", parentId: null })}><span className="suggestion-icon gold">◒</span><span><b>Get unstuck</b><small>Break down a problem</small></span></button>
      </div>
    </div>}

    {hasMessages && <AssistantRuntimeProvider runtime={runtime}>
      <ThreadPrimitive.Root className="chat-thread-root">
        <ThreadPrimitive.Viewport className="chat-thread" turnAnchor="bottom">
          <ThreadPrimitive.Messages>{({ message }) => {
            const item = messageList.find((candidate) => candidate.id === message.id);
            const original = detail?.messages.find((candidate) => candidate.id === message.id);
            const siblings = original && detail ? siblingsFor(detail.messages, original) : [];
            const branchIndex = siblings.findIndex((candidate) => candidate.id === message.id);
            const user = message.role === "user";
            const editing = user && message.composer.isEditing;
            return <MessagePrimitive.Root key={message.id} className="chat-message" data-role={message.role} data-running={!user && item?.generationStatus === "streaming" ? "true" : "false"}>
              <div className="chat-message-role">{user ? "YOU" : "SPRINTER"}{!user && item?.generationStatus === "streaming" && <span> {message.content ? "STREAMING" : "THINKING…"}</span>}</div>
              {editing ? <ComposerPrimitive.Root className="composer-card chat-composer chat-edit-composer">
                <ComposerPrimitive.Input aria-label="Message" placeholder="Edit message…" rows={2} />
                <div className="composer-toolbar"><span className="enter-hint">Press enter to save</span><div className="composer-right"><ComposerPrimitive.Cancel className="edit-cancel-button">Cancel</ComposerPrimitive.Cancel><ComposerPrimitive.Send className="send-button" aria-label="Save edited message"><ArrowUpRight size={17} /></ComposerPrimitive.Send></div></div>
              </ComposerPrimitive.Root> : <div className="chat-message-content"><MessagePrimitive.Parts /></div>}
              {!user && item?.generationStatus === "error" && <div className="chat-message-error" role="alert">{item.error ?? "The response could not be completed."}</div>}
              {!user && ["cancelled", "interrupted"].includes(item?.generationStatus ?? "") && <div className="chat-message-state">{item?.generationStatus === "cancelled" ? "Stopped" : "Interrupted"}. You can retry this response.</div>}
              <div className="chat-message-tools">
                {user && <ActionBarPrimitive.Root><ActionBarPrimitive.Edit>Edit</ActionBarPrimitive.Edit></ActionBarPrimitive.Root>}
                {!user && item?.generationStatus !== "streaming" && <>
                  <button type="button" onClick={() => retry.mutate({ messageId: message.id, model: retryModels[message.id] || undefined })} disabled={retry.isPending}><RotateCw size={12} /> Retry</button>
                  <select aria-label={`Retry model for message ${message.id}`} value={retryModels[message.id] ?? ""} onChange={(event) => setRetryModels((current) => ({ ...current, [message.id]: event.target.value }))}>
                    <option value="">Same model</option>
                    {modelQuery.data?.items.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}
                  </select>
                </>}
                {siblings.length > 1 && <div className="message-branch-picker" aria-label={`${user ? "User" : "Assistant"} branch`}>
                  <button type="button" aria-label={`Previous branch for message ${message.id}`} disabled={branchIndex <= 0 || switchMutation.isPending} onClick={() => switchMutation.mutate(siblings[branchIndex - 1].id)}>‹</button>
                  <span>{branchIndex + 1} / {siblings.length}</span>
                  <button type="button" aria-label={`Next branch for message ${message.id}`} disabled={branchIndex >= siblings.length - 1 || switchMutation.isPending} onClick={() => switchMutation.mutate(siblings[branchIndex + 1].id)}>›</button>
                </div>}
              </div>
            </MessagePrimitive.Root>;
          }}</ThreadPrimitive.Messages>
        </ThreadPrimitive.Viewport>
      </ThreadPrimitive.Root>
    </AssistantRuntimeProvider>}
    {streamingMessage && !streamingMessage.content && <div className="chat-thinking" role="status">THINKING…</div>}

    <div className="composer-wrap">
      {draftError && <div className="chat-send-error" role="alert">{draftError}{draftError.toLowerCase().includes("api key") && <a href="/settings">Open Settings</a>}</div>}
      <AssistantRuntimeProvider runtime={runtime}>
        <ComposerPrimitive.Root className="composer-card chat-composer">
          <ComposerPrimitive.Input aria-label="Message" placeholder="Message Sprinter…" rows={1} submitMode={coarsePointer ? "ctrlEnter" : "enter"} disabled={Boolean(streamingMessage) || send.isPending} />
          <div className="composer-toolbar"><div className="composer-left">{chatId && detail ? <ModelPicker models={modelQuery.data?.items ?? []} value={detail.model} favorites={settingsQuery.data?.favorite_models ?? []} isDisabled={modelQuery.isLoading || chatModelMutation.isPending} onChange={(model) => chatModelMutation.mutate(model)} placeholder="Choose model" /> : <DefaultModelPicker />}</div><div className="composer-right"><span className="enter-hint">Press enter to send</span>{streamingMessage ? <ComposerPrimitive.Cancel className="stop-button"><Square size={14} /> Stop</ComposerPrimitive.Cancel> : <ComposerPrimitive.Send className="send-button" aria-label="Send message"><ArrowUpRight size={17} /></ComposerPrimitive.Send>}</div></div>
        </ComposerPrimitive.Root>
      </AssistantRuntimeProvider>
      <p className="composer-caption">Sprinter can make mistakes. Check important information.</p>
    </div>
  </section>;
}
