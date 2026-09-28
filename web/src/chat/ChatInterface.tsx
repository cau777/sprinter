import { useEffect, useMemo, useRef, useState } from "react";
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
import { ArrowUpRight, Copy, FileText, Image as ImageIcon, Paperclip, RotateCw, Sparkles, Square, X } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { cancelMessage, fetchChat, regenerateMessage, sendToChat, sendToNewChat, switchBranch, watchMessage } from "../api/chats";
import type { ChatDetail, ChatMessage, ChatSummary } from "../api/chats";
import { ApiError } from "../api/client";
import { fetchModels, fetchSettings } from "../api/settings";
import { DefaultModelPicker } from "../components/DefaultModelPicker";
import { siblingsFor, visiblePath } from "./branch";
import { MarkdownText } from "./MarkdownText";
import { deleteUpload, uploadFile, type UploadRecord } from "../api/uploads";
import { useOnlineStatus } from "../api/useOnlineStatus";

type Props = { chatId?: string; messageId?: string };
type RuntimeMessage = ThreadMessageLike & { id: string; parentId: string | null; generationStatus: ChatMessage["status"]; error?: string | null; model?: string | null };
type SendResult = { chatId: string; chat?: ChatSummary; user_message: ChatMessage; assistant_message: ChatMessage };
type PendingUpload = { key: string; filename?: string; record?: UploadRecord | ChatMessage["attachments"][number]; progress: number; error?: string; uploading: boolean; persisted?: boolean };

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
      if (event.event === "done") {
        const usage = event.usage as { prompt_tokens?: number | null; completion_tokens?: number | null; reasoning_tokens?: number | null } | null | undefined;
        return {
          ...message,
          status: String(event.status ?? "complete") as ChatMessage["status"],
          finish_reason: (event.finish_reason as string | null) ?? null,
          prompt_tokens: usage?.prompt_tokens ?? message.prompt_tokens,
          completion_tokens: usage?.completion_tokens ?? message.completion_tokens,
          reasoning_tokens: usage?.reasoning_tokens ?? message.reasoning_tokens,
          cost: (event.cost as number | null) ?? message.cost,
        };
      }
      if (event.event === "error") return { ...message, status: "error", error: String(event.message ?? "Generation failed") };
      return message;
    }) };
  });
}

export function ChatInterface({ chatId, messageId }: Props) {
  const queryClient = useQueryClient();
  const navigate = useNavigate();
  const online = useOnlineStatus();
  const chatQuery = useQuery({ queryKey: ["chat", chatId], queryFn: () => fetchChat(chatId!), enabled: Boolean(chatId) });
  const settingsQuery = useQuery({ queryKey: ["settings"], queryFn: fetchSettings });
  const modelQuery = useQuery({ queryKey: ["models"], queryFn: fetchModels });
  const [coarsePointer, setCoarsePointer] = useState(false);
  const [draftError, setDraftError] = useState<string>();
  const [retryModels, setRetryModels] = useState<Record<string, string>>({});
  const [copiedMessageId, setCopiedMessageId] = useState<string | null>(null);
  const [pendingUploads, setPendingUploads] = useState<PendingUpload[]>([]);
  const [pdfEngine, setPdfEngine] = useState("");
  const [fileError, setFileError] = useState<string>();
  const [searchHighlight, setSearchHighlight] = useState<string | null>(null);
  const handledSearchTarget = useRef<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const detail = chatQuery.data;
  const canSendImages = (modelQuery.data?.items.find((model) => model.id === (detail?.model ?? settingsQuery.data?.default_model))?.input_modalities ?? []).some((modality) => modality.toLowerCase() === "image");

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

  const addFiles = async (files: FileList | File[]) => {
    if (!online) {
      setFileError("You’re offline. Reconnect to upload attachments.");
      return;
    }
    setFileError(undefined);
    const chosen = Array.from(files);
    const limit = settingsQuery.data?.upload_limits.files_per_message ?? 10;
    const room = Math.max(0, limit - pendingUploads.length);
    if (chosen.length > room) setFileError(`You can attach up to ${limit} files to a message.`);
    for (const file of chosen.slice(0, room)) {
      if ((file.type.startsWith("image/") || /\.(png|jpe?g|webp|gif)$/i.test(file.name)) && !canSendImages) {
        setFileError("The selected model cannot view images. Choose a model with image input to attach pictures.");
        continue;
      }
      const localKey = crypto.randomUUID();
      setPendingUploads((current) => [...current, { key: localKey, filename: file.name, progress: 0, uploading: true }]);
      try {
        const record = await uploadFile(file, (progress) => setPendingUploads((current) => current.map((item) => item.key === localKey ? { ...item, progress } : item)));
        setPendingUploads((current) => current.map((item) => item.key === localKey ? { ...item, record, progress: 100, uploading: false } : item));
      } catch (error) {
        setPendingUploads((current) => current.map((item) => item.key === localKey ? { ...item, uploading: false, error: error instanceof Error ? error.message : "Upload failed." } : item));
      }
    }
  };
  const removeUpload = (item: PendingUpload) => {
    setPendingUploads((current) => current.filter((candidate) => candidate.key !== item.key));
    if (online && item.record && !item.persisted) void deleteUpload(item.record.upload_id).catch(() => undefined);
  };
  const beginEdit = (message: ChatMessage) => {
    setPendingUploads(message.attachments.map((attachment) => ({ key: attachment.upload_id, filename: attachment.filename, record: attachment, progress: 100, uploading: false, persisted: true })));
    setFileError(undefined);
  };
  const copyMessage = async (message: ChatMessage) => {
    try {
      await navigator.clipboard.writeText(message.content);
      setCopiedMessageId(message.id);
      window.setTimeout(() => setCopiedMessageId((current) => current === message.id ? null : current), 1800);
    } catch {
      setCopiedMessageId(null);
    }
  };

  const send = useMutation({
    mutationFn: async ({ content, parentId, attachmentIds, engine }: { content: string; parentId: string | null; attachmentIds: string[]; engine?: string }): Promise<SendResult> => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to send messages.");
      if (chatId) {
        const response = await sendToChat(chatId, parentId, content, undefined, attachmentIds, engine);
        return { chatId, user_message: response.user_message, assistant_message: response.assistant_message };
      }
      const response = await sendToNewChat(content, undefined, attachmentIds, engine);
      return { chatId: response.chat.id, chat: response.chat, user_message: response.user_message, assistant_message: response.assistant_message };
    },
    onSuccess: ({ chatId: nextChatId, chat, user_message, assistant_message }) => {
      setDraftError(undefined);
      setPendingUploads([]);
      setFileError(undefined);
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
    onError: (error) => setDraftError(error instanceof ApiError ? error.message : error instanceof Error ? error.message : "Could not send this message."),
  });
  const submit = (content: string, parentId: string | null) => {
    if (!online) {
      setDraftError("You’re offline. Reconnect to send messages.");
      return;
    }
    const attachments = pendingUploads.map((item) => item.record).filter((item): item is NonNullable<typeof item> => Boolean(item));
    if (pendingUploads.some((item) => item.uploading || item.error) || attachments.length !== pendingUploads.length) {
      setFileError("Wait for uploads to finish or remove files that failed to upload.");
      return;
    }
    if (!content && !attachments.length) return;
    send.mutate({ content, parentId, attachmentIds: attachments.map((item) => item.upload_id), engine: pdfEngine || undefined });
  };

  const retry = useMutation({
    mutationFn: ({ messageId, model }: { messageId: string; model?: string }) => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to retry responses.");
      return regenerateMessage(messageId, model);
    },
    onSuccess: ({ assistant_message }) => {
      if (!chatId) return;
      queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => current ? { ...current, current_leaf_id: assistant_message.id, messages: [...current.messages, assistant_message] } : current);
    },
    onError: (error) => setDraftError(error instanceof ApiError ? error.message : "Could not retry this response."),
  });
  const switchMutation = useMutation({
    mutationFn: (messageId: string) => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to switch branches.");
      return switchBranch(chatId!, messageId);
    },
    onSuccess: ({ current_leaf_id }) => {
      queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => current ? { ...current, current_leaf_id } : current);
    },
    onError: (error) => setDraftError(error instanceof ApiError ? error.message : "Could not switch branches."),
  });

  useEffect(() => {
    if (!chatId || !messageId || !detail) return;
    const targetKey = `${chatId}:${messageId}`;
    if (handledSearchTarget.current === targetKey) return;
    const target = detail.messages.find((message) => message.id === messageId);
    if (!target) {
      handledSearchTarget.current = targetKey;
      return;
    }
    const path = visiblePath(detail.messages, detail.current_leaf_id);
    if (path.some((message) => message.id === messageId)) {
      handledSearchTarget.current = targetKey;
      return;
    }
    if (!online) return;

    handledSearchTarget.current = targetKey;
    void switchBranch(chatId, messageId).then(({ current_leaf_id }) => {
      queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => current ? { ...current, current_leaf_id } : current);
    }).catch((error: unknown) => {
      handledSearchTarget.current = null;
      setDraftError(error instanceof ApiError ? error.message : "Could not open the matching message branch.");
    });
  }, [chatId, detail, messageId, online, queryClient]);

  useEffect(() => {
    if (!chatId || !messageId || !visibleMessages.some((message) => message.id === messageId)) return;
    const element = document.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(messageId)}"]`);
    if (!element) return;
    setSearchHighlight(messageId);
    element.scrollIntoView({ behavior: "smooth", block: "center" });
    element.focus({ preventScroll: true });
    const timeout = window.setTimeout(() => setSearchHighlight((current) => current === messageId ? null : current), 2400);
    return () => window.clearTimeout(timeout);
  }, [chatId, messageId, visibleMessages]);

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
      submit(content, message.parentId ?? detail?.current_leaf_id ?? null);
    },
    onEdit: async (message) => {
      const content = textOf(message).trim();
      submit(content, message.parentId ?? null);
    },
    onReload: async (parentId) => {
      const original = visibleMessages.find((message) => message.role === "assistant" && message.parent_id === parentId);
      if (original) retry.mutate({ messageId: original.id });
    },
    onCancel: async () => { if (online && streamingMessage) await cancelMessage(streamingMessage.id).catch(() => undefined); },
  });

  if (chatId && chatQuery.isLoading) return <section className="chat-loading">Opening conversation…</section>;
  if (chatId && chatQuery.isError && !chatQuery.data) return <section className="chat-loading" role="alert">{chatQuery.error.message}</section>;
  const hasMessages = visibleMessages.length > 0;

  return <section className={`conversation-stage ${hasMessages ? "has-messages" : ""}`} onPaste={(event) => { const files = Array.from(event.clipboardData.files); if (files.length) { event.preventDefault(); void addFiles(files.map((file) => file.type.startsWith("image/") ? pastedImageName(file) : file)); } }} onDragOver={(event) => { if (online && Array.from(event.dataTransfer.types).includes("Files")) event.preventDefault(); }} onDrop={(event) => { if (event.dataTransfer.files.length) { event.preventDefault(); void addFiles(event.dataTransfer.files); } }}>
    <input ref={fileInput} className="attachment-file-input" type="file" multiple disabled={!online} accept={`${canSendImages ? "image/png,image/jpeg,image/webp,image/gif," : ""}.pdf,.txt,.md,.csv,.json,.yaml,.yml,.toml,.xml,.html,.css,.js,.ts,.tsx,.jsx,.rs,.py,.go,.sh,.sql,.log`} onChange={(event) => { if (event.currentTarget.files) void addFiles(event.currentTarget.files); event.currentTarget.value = ""; }} />
    {!hasMessages && <div className="welcome-content">
      <div className="welcome-icon"><Sparkles size={22} /></div>
      <p className="eyebrow">A CLEARER WAY TO THINK</p>
      <h1>What’s on your mind<span>?</span></h1>
      <p className="welcome-copy">A thought, a question, a half-formed idea.<br />Start anywhere. We’ll take it from there.</p>
      <div className="prompt-suggestions">
        <button className="suggestion-card" type="button" disabled={!online} onClick={() => submit("Help me think through an idea.", null)}><span className="suggestion-icon violet">✳</span><span><b>Think it through</b><small>Help me explore an idea</small></span></button>
        <button className="suggestion-card" type="button" disabled={!online} onClick={() => submit("Help me make something.", null)}><span className="suggestion-icon blue">⌘</span><span><b>Make something</b><small>Write, plan, or create</small></span></button>
        <button className="suggestion-card" type="button" disabled={!online} onClick={() => submit("Help me get unstuck.", null)}><span className="suggestion-icon gold">◒</span><span><b>Get unstuck</b><small>Break down a problem</small></span></button>
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
            return <MessagePrimitive.Root key={message.id} className="chat-message" data-message-id={message.id} data-search-target={searchHighlight === message.id ? "true" : undefined} data-message-status={item?.generationStatus ?? "complete"} tabIndex={-1} data-role={message.role} data-running={!user && item?.generationStatus === "streaming" ? "true" : "false"}>
              <div className="chat-message-role">{user ? "YOU" : "SPRINTER"}{!user && item?.generationStatus === "streaming" && <span> {message.content ? "STREAMING" : "THINKING…"}</span>}</div>
              {editing ? <ComposerPrimitive.Root className="composer-card chat-composer chat-edit-composer">
                <ComposerPrimitive.Input aria-label="Message" placeholder="Edit message…" rows={2} disabled={!online} />
                <UploadChips items={pendingUploads} onRemove={removeUpload} />
                <div className="composer-toolbar"><button type="button" className="attach-button" onClick={() => fileInput.current?.click()} aria-label="Attach files" disabled={!online}><Paperclip size={14} /> Add files</button><span className="enter-hint">Press enter to save</span><div className="composer-right"><ComposerPrimitive.Cancel className="edit-cancel-button" onClick={() => setPendingUploads([])}>Cancel</ComposerPrimitive.Cancel><ComposerPrimitive.Send className="send-button" aria-label="Save edited message" disabled={!online}><ArrowUpRight size={17} /></ComposerPrimitive.Send></div></div>
              </ComposerPrimitive.Root> : <div className="chat-message-content"><MessagePrimitive.Parts components={{ Text: MarkdownText }} /></div>}
              {user && !editing && original?.attachments.length ? <UploadChips items={original.attachments.map((record) => ({ key: record.upload_id, record, progress: 100, uploading: false, persisted: true }))} /> : null}
              {!user && item?.generationStatus === "error" && <div className="chat-message-error" role="alert">{item.error ?? "The response could not be completed."}</div>}
              {!user && ["cancelled", "interrupted"].includes(item?.generationStatus ?? "") && <div className="chat-message-state">{item?.generationStatus === "cancelled" ? "Stopped" : "Interrupted"}. You can retry this response.</div>}
              <div className="chat-message-tools">
                {user && original && <>
                  {online && <ActionBarPrimitive.Root><ActionBarPrimitive.Edit onClick={() => beginEdit(original)}>Edit</ActionBarPrimitive.Edit></ActionBarPrimitive.Root>}
                  <button type="button" aria-label={copiedMessageId === message.id ? "Message copied" : "Copy message"} onClick={() => void copyMessage(original)}><Copy size={12} /> {copiedMessageId === message.id ? "Copied" : "Copy"}</button>
                </>}
                {!user && item?.generationStatus !== "streaming" && <>
                  {original && <button type="button" aria-label={copiedMessageId === message.id ? "Message copied" : "Copy message"} onClick={() => void copyMessage(original)}><Copy size={12} /> {copiedMessageId === message.id ? "Copied" : "Copy"}</button>}
                  <button type="button" onClick={() => retry.mutate({ messageId: message.id, model: retryModels[message.id] || undefined })} disabled={!online || retry.isPending}><RotateCw size={12} /> Retry</button>
                  <select aria-label={`Retry model for message ${message.id}`} value={retryModels[message.id] ?? ""} disabled={!online} onChange={(event) => setRetryModels((current) => ({ ...current, [message.id]: event.target.value }))}>
                    <option value="">Same model</option>
                    {modelQuery.data?.items.map((model) => <option key={model.id} value={model.id}>{model.name}</option>)}
                  </select>
                </>}
                {siblings.length > 1 && <div className="message-branch-picker" aria-label={`${user ? "User" : "Assistant"} branch`}>
                  <button type="button" aria-label={`Previous branch for message ${message.id}`} disabled={!online || branchIndex <= 0 || switchMutation.isPending} onClick={() => switchMutation.mutate(siblings[branchIndex - 1].id)}>‹</button>
                  <span>{branchIndex + 1} / {siblings.length}</span>
                  <button type="button" aria-label={`Next branch for message ${message.id}`} disabled={!online || branchIndex >= siblings.length - 1 || switchMutation.isPending} onClick={() => switchMutation.mutate(siblings[branchIndex + 1].id)}>›</button>
                </div>}
                {!user && original && (original.prompt_tokens != null || original.completion_tokens != null || original.cost != null) && <span className="message-usage" aria-label={usageLabel(original)}>{usageSummary(original)}</span>}
              </div>
            </MessagePrimitive.Root>;
          }}</ThreadPrimitive.Messages>
        </ThreadPrimitive.Viewport>
      </ThreadPrimitive.Root>
    </AssistantRuntimeProvider>}
    {streamingMessage && !streamingMessage.content && <div className="chat-thinking" role="status">THINKING…</div>}

    <div className="composer-wrap">
      {draftError && <div className="chat-send-error" role="alert">{draftError}{draftError.toLowerCase().includes("api key") && <a href="/settings">Open Settings</a>}</div>}
      {fileError && <div className="chat-send-error" role="alert">{fileError}<button type="button" onClick={() => setFileError(undefined)} aria-label="Dismiss upload error"><X size={12} /></button></div>}
      {!canSendImages && visibleMessages.some((message) => message.attachments.some((attachment) => attachment.kind === "image")) && <div className="attachment-warning" role="status">This model cannot read images attached earlier in this conversation.</div>}
      <AssistantRuntimeProvider runtime={runtime}>
        <ComposerPrimitive.Root className="composer-card chat-composer">
          <ComposerPrimitive.Input aria-label="Message" placeholder="Message Sprinter…" rows={1} submitMode={coarsePointer ? "ctrlEnter" : "enter"} disabled={!online || Boolean(streamingMessage) || send.isPending} />
          <UploadChips items={pendingUploads} onRemove={removeUpload} />
          {(pendingUploads.some((item) => item.record?.kind === "pdf") || visibleMessages.some((message) => message.attachments.some((attachment) => attachment.kind === "pdf"))) && <label className="pdf-engine-control">PDF engine <select aria-label="PDF engine for this send" value={pdfEngine} disabled={!online} onChange={(event) => setPdfEngine(event.target.value)}><option value="">Settings default ({settingsQuery.data?.pdf_engine ?? "cloudflare-ai"})</option><option value="cloudflare-ai">Cloudflare AI</option><option value="mistral-ocr">Mistral OCR</option><option value="native">Native</option></select></label>}
          <div className="composer-toolbar"><div className="composer-left"><button type="button" className="attach-button" onClick={() => fileInput.current?.click()} aria-label="Attach files" disabled={!online || Boolean(streamingMessage) || send.isPending}><Paperclip size={14} /> Attach</button>{!chatId && <DefaultModelPicker isDisabled={!online} />}</div><div className="composer-right"><span className="enter-hint">{online ? "Press enter to send" : "Reconnect to send"}</span>{streamingMessage ? <ComposerPrimitive.Cancel className="stop-button" disabled={!online}><Square size={14} /> Stop</ComposerPrimitive.Cancel> : <ComposerPrimitive.Send className="send-button" aria-label="Send message" disabled={!online || pendingUploads.some((item) => item.uploading)}><ArrowUpRight size={17} /></ComposerPrimitive.Send>}</div></div>
        </ComposerPrimitive.Root>
      </AssistantRuntimeProvider>
      <p className="composer-caption">Sprinter can make mistakes. Check important information.</p>
    </div>
  </section>;
}

function UploadChips({ items, onRemove }: { items: PendingUpload[]; onRemove?: (item: PendingUpload) => void }) {
  if (!items.length) return null;
  return <div className="chat-attachment-list" aria-label="Attachments">
    {items.map((item) => {
      const file = item.record;
      const id = file?.upload_id;
      const filename = file?.filename ?? item.filename ?? "Uploading file";
      const image = file?.kind === "image";
      return <div className={`chat-attachment-chip ${item.error ? "has-error" : ""}`} key={item.key}>
        {image && id ? onRemove ? <img className="chat-attachment-thumb" src={`/api/uploads/${encodeURIComponent(id)}`} alt="" /> : <a href={`/api/uploads/${encodeURIComponent(id)}`} target="_blank" rel="noreferrer" aria-label={`Open ${filename}`}><img className="chat-attachment-thumb" src={`/api/uploads/${encodeURIComponent(id)}`} alt="" /></a> : <span className="chat-attachment-icon">{file?.kind === "pdf" || file?.kind === "text" ? <FileText size={14} /> : <ImageIcon size={14} />}</span>}
        <span className="chat-attachment-name" title={filename}>{filename}<small>{item.error ?? (item.uploading ? `Uploading ${item.progress}%` : file ? `${formatBytes(file.size)} · ${file.kind}` : "Preparing upload…")}</small></span>
        {item.uploading && <span className="chat-upload-progress" style={{ width: `${Math.max(4, item.progress)}%` }} />}
        {onRemove && <button type="button" className="chat-attachment-remove" onClick={() => onRemove(item)} aria-label={`Remove ${filename}`}><X size={13} /></button>}
        {id && !onRemove && !image && <a className="chat-attachment-open" href={`/api/uploads/${encodeURIComponent(id)}`} target="_blank" rel="noreferrer" aria-label={`Open ${filename}`}>Open</a>}
      </div>;
    })}
  </div>;
}

function formatBytes(size: number) {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(0)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function usageSummary(message: ChatMessage) {
  const tokenCount = message.prompt_tokens != null || message.completion_tokens != null
    ? new Intl.NumberFormat().format((message.prompt_tokens ?? 0) + (message.completion_tokens ?? 0))
    : null;
  const cost = message.cost == null ? null : `$${message.cost.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 6 })}`;
  return [tokenCount == null ? null : `${tokenCount} tokens`, cost].filter(Boolean).join(" · ");
}

function usageLabel(message: ChatMessage) {
  return `OpenRouter usage: ${usageSummary(message)}`;
}

function pastedImageName(file: File) {
  const now = new Date();
  const pad = (part: number) => String(part).padStart(2, "0");
  const stamp = `${now.getFullYear()}${pad(now.getMonth() + 1)}${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}${pad(now.getSeconds())}`;
  const ext = file.type === "image/jpeg" ? "jpg" : file.type === "image/gif" ? "gif" : file.type === "image/webp" ? "webp" : "png";
  return new File([file], `pasted-${stamp}.${ext}`, { type: file.type || "image/png" });
}
