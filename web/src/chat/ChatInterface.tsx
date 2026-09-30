import { createContext, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
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
import { ArrowUpRight, Copy, FileText, Globe, Image as ImageIcon, Paperclip, Plus, RotateCw, Sparkles, Square, SquareTerminal, X } from "lucide-react";
import { useNavigate } from "@tanstack/react-router";
import { cancelMessage, fetchChat, regenerateMessage, sendToChat, sendToNewChat, switchBranch, updateChat, watchMessage } from "../api/chats";
import type { ChatDetail, ChatMessage, ChatSummary } from "../api/chats";
import { ApiError } from "../api/client";
import { fetchModels, fetchSettings } from "../api/settings";
import { DefaultModelPicker } from "../components/DefaultModelPicker";
import { siblingsFor, visiblePath } from "./branch";
import { MarkdownText } from "./MarkdownText";
import { deleteUpload, fetchPdfFile, storePdfText, uploadFile, type PdfTextStats, type UploadRecord } from "../api/uploads";
import { extractPdfText, isPasswordProtectedPdfError, type ExtractedPdfText, pdfExtractionErrorMessage } from "../pdf/extractText";
import { reportClientError } from "../clientErrors";
import { useOnlineStatus } from "../api/useOnlineStatus";
import { Button, Tooltip } from "@heroui/react";
import { SelectField } from "../components/SelectField";
import { WEB_SEARCH, BASH, toolLabel } from "./toolCatalog";
import { useMessagePartText } from "@assistant-ui/react";
import { MarkdownContent } from "./MarkdownText";
import type { Citation, ToolStep } from "../api/types.gen";

type Props = { chatId?: string; messageId?: string };
type RuntimeMessage = ThreadMessageLike & { id: string; parentId: string | null; generationStatus: ChatMessage["status"]; error?: string | null; model?: string | null };
type SendResult = { chatId: string; chat?: ChatSummary; user_message: ChatMessage; assistant_message: ChatMessage };
type PendingUpload = { key: string; filename?: string; record?: UploadRecord | ChatMessage["attachments"][number]; progress: number; error?: string; uploading: boolean; persisted?: boolean; extracting?: boolean; pageProgress?: { page: number; pages: number }; extractedText?: ExtractedPdfText };
const ToolMessageContext = createContext<ChatMessage | undefined>(undefined);

function textOf(message: AppendMessage) {
  return message.content.filter((part) => part.type === "text").map((part) => part.text).join("");
}

function applyMessageEvent(queryClient: ReturnType<typeof useQueryClient>, chatId: string, messageId: string, event: { event: string; [key: string]: unknown }) {
  if (event.event === "title") {
    queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => current ? { ...current, title: String(event.title) } : current);
    void queryClient.invalidateQueries({ queryKey: ["chats"] });
    return;
  }
  if (event.event === "done" && event.tool_fallback === true) {
    void queryClient.invalidateQueries({ queryKey: ["models"] });
  }
  queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => {
    if (!current) return current;
    return { ...current, messages: current.messages.map((message) => {
      if (message.id !== messageId) return message;
      if (event.event === "snapshot") return {
        ...message,
        content: String(event.content ?? ""),
        tools: (event.tools as string[] | undefined) ?? message.tools,
        citations: (event.citations as Citation[] | undefined) ?? message.citations,
        tool_steps: (event.tool_steps as ToolStep[] | undefined) ?? message.tool_steps,
        web_search_requests: (event.web_search_requests as number | null | undefined) ?? message.web_search_requests,
        tool_cost: (event.tool_cost as number | null | undefined) ?? message.tool_cost,
        tool_fallback: Boolean(event.tool_fallback ?? message.tool_fallback),
      };
      if (event.event === "delta") return { ...message, content: message.content + String(event.content ?? "") };
      if (event.event === "citations") {
        const incoming = (event.items as Citation[] | undefined) ?? [];
        const current = message.citations ?? [];
        const citations = [...current];
        for (const citation of incoming) if (!citations.some((item) => item.url === citation.url)) citations.push(citation);
        return { ...message, citations };
      }
      if (event.event === "tool_step") {
        const step = event.step as ToolStep | undefined;
        if (!step) return message;
        const tool_steps = [...(message.tool_steps ?? [])];
        const index = tool_steps.findIndex((item) => item.id === step.id);
        if (index < 0) tool_steps.push(step);
        else tool_steps[index] = step;
        return { ...message, tool_steps };
      }
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
          web_search_requests: (event.web_search_requests as number | null) ?? message.web_search_requests,
          tool_cost: (event.tool_cost as number | null) ?? message.tool_cost,
          tool_fallback: Boolean(event.tool_fallback ?? message.tool_fallback),
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
  const [fileError, setFileError] = useState<string>();
  const [searchHighlight, setSearchHighlight] = useState<string | null>(null);
  const [draftTools, setDraftTools] = useState<string[]>([]);
  const [savingTools, setSavingTools] = useState(false);
  const handledSearchTarget = useRef<string | null>(null);
  const fileInput = useRef<HTMLInputElement>(null);
  const detail = chatQuery.data;
  const activeModel = modelQuery.data?.items.find((model) => model.id === (detail?.model ?? settingsQuery.data?.default_model));
  const canSendImages = (activeModel?.input_modalities ?? []).some((modality) => modality.toLowerCase() === "image");
  const canReadPdfs = (activeModel?.input_modalities ?? []).some((modality) => ["file", "pdf"].includes(modality.toLowerCase()));
  const selectedTools = detail?.tools ?? draftTools;
  const supportedTools = (activeModel?.tools ?? []).filter((tool) =>
    tool.coverage !== "none" && (tool.id === WEB_SEARCH || tool.id === BASH),
  );

  const toggleTool = (id: string) => {
    const next = selectedTools.includes(id) ? selectedTools.filter((tool) => tool !== id) : [id];
    if (!chatId) {
      setDraftTools(next);
      return;
    }
    const previous = queryClient.getQueryData<ChatDetail>(["chat", chatId]);
    queryClient.setQueryData<ChatDetail>(["chat", chatId], (current) => current ? { ...current, tools: next } : current);
    setSavingTools(true);
    void updateChat(chatId, { tools: next }).catch((error: unknown) => {
      if (previous) queryClient.setQueryData(["chat", chatId], previous);
      setDraftError(error instanceof ApiError ? error.message : "Could not save tool preferences.");
    }).finally(() => setSavingTools(false));
  };

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
      const isPdf = file.type === "application/pdf" || file.name.toLowerCase().endsWith(".pdf");
      const abortExtraction = new AbortController();
      let pageProgress: { page: number; pages: number } | undefined;
      setPendingUploads((current) => [...current, { key: localKey, filename: file.name, progress: 0, uploading: true, extracting: isPdf }]);
      const extraction = isPdf
        ? extractPdfText(file, (progress) => {
          pageProgress = progress;
          setPendingUploads((current) => current.map((item) => item.key === localKey ? { ...item, extracting: true, pageProgress: progress } : item));
        }, abortExtraction.signal).then((text) => ({ text }), (error: unknown) => ({ error }))
        : undefined;
      let record: UploadRecord | undefined;
      try {
        record = await uploadFile(file, (progress) => setPendingUploads((current) => current.map((item) => item.key === localKey ? { ...item, progress } : item)));
        if (record.kind === "pdf") {
          if (record.text) {
            abortExtraction.abort();
          } else {
            const extracted = await extraction;
            if (!extracted || "error" in extracted) {
              const error = extracted && "error" in extracted ? extracted.error : new Error("PDF extraction did not start");
              if (!isPasswordProtectedPdfError(error) && !(error instanceof DOMException && error.name === "AbortError")) {
                reportClientError({
                  level: "error",
                  message: `PDF extraction failed (error_name=${errorName(error)}, pages=${pageProgress?.pages ?? 0}, size=${file.size})`,
                  route: "/api/uploads",
                });
              }
              await deleteUpload(record.upload_id).catch(() => undefined);
              setPendingUploads((current) => current.filter((item) => item.key !== localKey));
              setFileError(pdfExtractionErrorMessage(error));
              continue;
            }
            setPendingUploads((current) => current.map((item) => item.key === localKey ? { ...item, record, extractedText: extracted.text, extracting: false } : item));
            let stats: PdfTextStats;
            try {
              stats = await storePdfText(record.upload_id, extracted.text);
            } catch {
              setPendingUploads((current) => current.map((item) => item.key === localKey ? {
                ...item,
                uploading: false,
                extracting: false,
                error: "Couldn't save PDF text. Retry.",
                extractedText: extracted.text,
              } : item));
              continue;
            }
            record = withPdfTextStats(record, stats);
          }
        }
        setPendingUploads((current) => current.map((item) => item.key === localKey ? { ...item, record, progress: 100, uploading: false, extracting: false, extractedText: undefined } : item));
      } catch (error) {
        abortExtraction.abort();
        setPendingUploads((current) => current.map((item) => item.key === localKey ? { ...item, uploading: false, error: error instanceof Error ? error.message : "Upload failed." } : item));
      }
    }
  };
  const retryPdfText = async (item: PendingUpload) => {
    if (!item.record || !item.extractedText || item.record.kind !== "pdf") return;
    setPendingUploads((current) => current.map((candidate) => candidate.key === item.key ? { ...candidate, uploading: true, error: undefined } : candidate));
    try {
      const stats = await storePdfText(item.record.upload_id, item.extractedText);
      const record = withPdfTextStats(item.record as UploadRecord, stats);
      setPendingUploads((current) => current.map((candidate) => candidate.key === item.key ? { ...candidate, record, uploading: false, error: undefined, extractedText: undefined } : candidate));
    } catch (error) {
      setPendingUploads((current) => current.map((candidate) => candidate.key === item.key ? { ...candidate, uploading: false, error: error instanceof Error ? error.message : "Could not save PDF text." } : candidate));
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

  const backfillMissingPdfs = async (uploadIds: string[]) => {
    setDraftError(`Reading ${uploadIds.length} earlier PDF${uploadIds.length === 1 ? "" : "s"}…`);
    try {
      await Promise.all(uploadIds.map(async (uploadId) => {
        const attachment = detail?.messages.flatMap((message) => message.attachments).find((item) => item.upload_id === uploadId);
        const file = await fetchPdfFile(uploadId, attachment?.filename ?? "attachment.pdf");
        let pages = 0;
        let extracted: ExtractedPdfText;
        try {
          extracted = await extractPdfText(file, (progress) => { pages = progress.pages; });
        } catch (error) {
          if (!isPasswordProtectedPdfError(error)) {
            reportClientError({ level: "error", message: `PDF extraction failed (error_name=${errorName(error)}, pages=${pages}, size=${file.size})`, route: "/api/uploads" });
          }
          throw new Error(isPasswordProtectedPdfError(error) ? "A password-protected earlier PDF can't be read." : "Couldn't read an earlier PDF.");
        }
        await storePdfText(uploadId, extracted);
      }));
      if (chatId) await queryClient.invalidateQueries({ queryKey: ["chat", chatId] });
      setDraftError(undefined);
    } catch (error) {
      setDraftError(error instanceof Error ? error.message : "Couldn't read earlier PDFs.");
      throw error;
    }
  };

  const send = useMutation({
    mutationFn: async ({ content, parentId, attachmentIds, tools }: { content: string; parentId: string | null; attachmentIds: string[]; tools: string[] }): Promise<SendResult> => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to send messages.");
      const request = async (): Promise<SendResult> => {
        if (chatId) {
          const response = await sendToChat(chatId, parentId, content, undefined, attachmentIds);
          return { chatId, user_message: response.user_message, assistant_message: response.assistant_message };
        }
        const response = await sendToNewChat(content, undefined, attachmentIds, tools);
        return { chatId: response.chat.id, chat: response.chat, user_message: response.user_message, assistant_message: response.assistant_message };
      };
      try {
        return await request();
      } catch (error) {
        if (!(error instanceof ApiError) || error.code !== "pdf_text_missing" || !error.uploadIds.length) throw error;
        await backfillMissingPdfs(error.uploadIds);
        return request();
      }
    },
    onSuccess: ({ chatId: nextChatId, chat, user_message, assistant_message }, variables) => {
      setDraftError(undefined);
      setPendingUploads([]);
      setFileError(undefined);
      queryClient.setQueryData<ChatDetail>(["chat", nextChatId], (current) => {
        if (current) return { ...current, current_leaf_id: assistant_message.id, messages: [...current.messages, user_message, assistant_message] };
        const now = Date.now();
        return {
          id: nextChatId, title: chat?.title ?? null, title_source: "auto", model: chat?.model ?? settingsQuery.data?.default_model ?? "", tools: variables.tools, current_leaf_id: assistant_message.id, created_at: now, updated_at: now,
          messages: [user_message, assistant_message],
        };
      });
      void queryClient.invalidateQueries({ queryKey: ["chats"] });
      if (!chatId) void navigate({ to: "/$chatId", params: { chatId: nextChatId }, search: { messageId: undefined } });
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
    send.mutate({ content, parentId, attachmentIds: attachments.map((item) => item.upload_id), tools: chatId ? [] : selectedTools });
  };

  const retry = useMutation({
    mutationFn: async ({ messageId, model }: { messageId: string; model?: string }) => {
      if (!navigator.onLine) throw new Error("You’re offline. Reconnect to retry responses.");
      try {
        return await regenerateMessage(messageId, model);
      } catch (error) {
        if (!(error instanceof ApiError) || error.code !== "pdf_text_missing" || !error.uploadIds.length) throw error;
        await backfillMissingPdfs(error.uploadIds);
        return regenerateMessage(messageId, model);
      }
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

  if (chatId && chatQuery.isLoading) return <section className="grid flex-1 place-items-center text-[11px] text-slate-400">Opening conversation…</section>;
  if (chatId && chatQuery.isError && !chatQuery.data) return <section className="grid flex-1 place-items-center text-[11px] text-slate-400" role="alert">{chatQuery.error.message}</section>;
  const hasMessages = visibleMessages.length > 0;
  const hasOmittedScan = !canReadPdfs && [
    ...pendingUploads.map((item) => item.record),
    ...visibleMessages.flatMap((message) => message.attachments),
  ].some((attachment) => attachment?.kind === "pdf"
    && attachment.text_pages != null
    && attachment.text_pages > 0
    && attachment.text_empty_pages === attachment.text_pages);

  return <section className={`flex min-h-0 flex-1 flex-col items-center justify-center px-[22px] pt-[18px] pb-[21px] max-[720px]:px-[15px] max-[720px]:pt-[18px] max-[720px]:pb-[calc(12px+env(safe-area-inset-bottom))] ${hasMessages ? "!items-stretch !justify-start !pt-[11px]" : ""}`} onPaste={(event) => { const files = Array.from(event.clipboardData.files); if (files.length) { event.preventDefault(); void addFiles(files.map((file) => file.type.startsWith("image/") ? pastedImageName(file) : file)); } }} onDragOver={(event) => { if (online && Array.from(event.dataTransfer.types).includes("Files")) event.preventDefault(); }} onDrop={(event) => { if (event.dataTransfer.files.length) { event.preventDefault(); void addFiles(event.dataTransfer.files); } }}>
    <input ref={fileInput} className="hidden" type="file" multiple disabled={!online} accept={`${canSendImages ? "image/png,image/jpeg,image/webp,image/gif," : ""}.pdf,.txt,.md,.csv,.json,.yaml,.yml,.toml,.xml,.html,.css,.js,.ts,.tsx,.jsx,.rs,.py,.go,.sh,.sql,.log`} onChange={(event) => { if (event.currentTarget.files) void addFiles(event.currentTarget.files); event.currentTarget.value = ""; }} />
    {!hasMessages && <div className="w-full max-w-[610px] -translate-y-[11px] text-center max-[720px]:-translate-y-[3px]">
      <div className="mx-auto mb-[21px] grid size-[52px] place-items-center rounded-[17px] border border-[rgba(61,232,255,.18)] bg-[linear-gradient(145deg,rgba(61,232,255,.11),rgba(61,232,255,.025))] text-[var(--accent)] shadow-[0_10px_42px_rgba(61,232,255,.07)] max-[720px]:mb-[17px] max-[720px]:size-[45px]"><Sparkles size={22} /></div>
      <p className="mb-[11px] font-mono text-[9px] tracking-[.16em] text-slate-400">A CLEARER WAY TO THINK</p>
      <h1 className="m-0 font-display text-[clamp(28px,3vw,38px)] font-medium leading-[1.3] tracking-[-.045em] text-slate-100 max-[720px]:text-[28px]">What’s on your mind<span className="text-[var(--accent)]">?</span></h1>
      <p className="mt-[11px] mb-[25px] text-[13px] leading-[1.75] text-[var(--text-muted)] max-[720px]:text-xs">A thought, a question, a half-formed idea.<br />Start anywhere. We’ll take it from there.</p>
    </div>}

    {hasMessages && <AssistantRuntimeProvider runtime={runtime}>
      <ThreadPrimitive.Root className="mx-auto flex min-h-0 w-full max-w-[790px] flex-1 flex-col">
        <ThreadPrimitive.Viewport className="min-h-0 flex-1 overflow-y-auto px-5 pt-2.5 pb-[18px] max-[720px]:px-0.5 max-[720px]:pt-1 max-[720px]:pb-3" turnAnchor="bottom">
          <ThreadPrimitive.Messages>{({ message }) => {
            const item = messageList.find((candidate) => candidate.id === message.id);
            const original = detail?.messages.find((candidate) => candidate.id === message.id);
            const siblings = original && detail ? siblingsFor(detail.messages, original) : [];
            const branchIndex = siblings.findIndex((candidate) => candidate.id === message.id);
            const user = message.role === "user";
            const editing = user && message.composer.isEditing;
            return <MessagePrimitive.Root key={message.id} className="chat-message mb-5 max-w-full border-b border-[rgba(120,160,220,.08)] px-0.5 pt-[19px] pb-4 data-[role=user]:ml-auto data-[role=user]:max-w-[88%] data-[role=user]:rounded-[14px] data-[role=user]:border data-[role=user]:border-[rgba(120,160,220,.11)] data-[role=user]:bg-[rgba(18,26,41,.65)] data-[role=user]:px-4 data-[role=user]:py-3.5 data-[role=assistant]:border-l-2 data-[role=assistant]:border-l-[var(--border)] data-[role=assistant]:pl-4 max-[720px]:data-[role=user]:max-w-[94%]" data-message-id={message.id} data-search-target={searchHighlight === message.id ? "true" : undefined} data-message-status={item?.generationStatus ?? "complete"} tabIndex={-1} data-role={message.role} data-running={!user && item?.generationStatus === "streaming" ? "true" : "false"}>
              <div className="mb-2 font-mono text-[8px] tracking-[.13em] text-slate-500">{user ? "YOU" : "SPRINTER"}{!user && item?.generationStatus === "streaming" && <span className="ml-[7px] text-[var(--accent)]"> {message.content ? "STREAMING" : searchProgressLabel(original)}</span>}</div>
              {editing ? <ComposerPrimitive.Root className="relative w-full rounded-[15px] border border-[rgba(120,160,220,.17)] bg-[rgba(19,26,41,.82)] px-3.5 pt-3.5 pb-2.5 shadow-[0_10px_44px_rgba(61,232,255,.055),0_18px_60px_rgba(0,0,0,.17)]">
                <ComposerPrimitive.Input className="block min-h-[31px] max-h-[170px] w-full resize-none border-0 bg-transparent px-0.5 pb-2 text-[13px] text-[var(--text)] outline-none placeholder:text-slate-500" aria-label="Message" placeholder="Edit message…" rows={2} disabled={!online} />
                <UploadChips items={pendingUploads} onRemove={removeUpload} onRetry={retryPdfText} contextLength={activeModel?.context_length} modelName={activeModel?.name} canReadPdfs={canReadPdfs} />
                <div className="flex items-center justify-between"><Button variant="secondary" onPress={() => fileInput.current?.click()} aria-label="Attach files" isDisabled={!online}><Paperclip size={14} /> Add files</Button><span className="flex items-center gap-1 text-[9px] text-slate-500 max-[720px]:hidden">Press enter to save</span><div className="flex items-center gap-[9px]"><ComposerPrimitive.Cancel className="composer-action-danger" onClick={() => setPendingUploads([])}>Cancel</ComposerPrimitive.Cancel><ComposerPrimitive.Send className="composer-action-primary" aria-label="Save edited message" disabled={!online}><ArrowUpRight size={17} /></ComposerPrimitive.Send></div></div>
              </ComposerPrimitive.Root> : <div className="chat-message-content"><ToolMessageContext.Provider value={original}><MessagePrimitive.Parts components={{ Text: user ? MarkdownText : ToolAwareMarkdown }} /></ToolMessageContext.Provider></div>}
              {user && !editing && original?.attachments.length ? <UploadChips items={original.attachments.map((record) => ({ key: record.upload_id, record, progress: 100, uploading: false, persisted: true }))} contextLength={activeModel?.context_length} modelName={activeModel?.name} canReadPdfs={canReadPdfs} /> : null}
              {!user && original && <CitationChips citations={original.citations ?? []} />}
              {!user && original?.tool_fallback && <div className="mt-2 rounded-md border border-amber-300/20 bg-amber-300/[.06] px-2 py-1.5 text-[10px] text-amber-100" role="status">Search was handled by a third-party engine.</div>}
              {!user && item?.generationStatus === "error" && <div className="mt-[9px] text-[10px] text-rose-300" role="alert">{item.error ?? "The response could not be completed."}</div>}
              {!user && ["cancelled", "interrupted"].includes(item?.generationStatus ?? "") && <div className="mt-2 text-[9px] text-slate-400">{item?.generationStatus === "cancelled" ? "Stopped" : "Interrupted"}. You can retry this response.</div>}
              <div className="chat-message-tools mt-[9px] flex items-center gap-[9px] text-slate-500">
                {user && original && <>
                  {online && <ActionBarPrimitive.Root><ActionBarPrimitive.Edit onClick={() => beginEdit(original)}>Edit</ActionBarPrimitive.Edit></ActionBarPrimitive.Root>}
                  <Button variant="ghost" className="gap-1 rounded px-1.5 py-1 text-[9px] text-slate-400 hover:text-[var(--accent)]" aria-label={copiedMessageId === message.id ? "Message copied" : "Copy message"} onPress={() => void copyMessage(original)}><Copy size={12} /> {copiedMessageId === message.id ? "Copied" : "Copy"}</Button>
                </>}
                {!user && item?.generationStatus !== "streaming" && <>
                  {original && <Button variant="ghost" className="gap-1 rounded px-1.5 py-1 text-[9px] text-slate-400 hover:text-[var(--accent)]" aria-label={copiedMessageId === message.id ? "Message copied" : "Copy message"} onPress={() => void copyMessage(original)}><Copy size={12} /> {copiedMessageId === message.id ? "Copied" : "Copy"}</Button>}
                  <Button variant="ghost" className="gap-1 rounded px-1.5 py-1 text-[9px] text-slate-400 hover:text-[var(--accent)]" onPress={() => retry.mutate({ messageId: message.id, model: retryModels[message.id] || undefined })} isDisabled={!online || retry.isPending}><RotateCw size={12} /> Retry</Button>
                  <SelectField aria-label={`Retry model for message ${message.id}`} className="max-w-40" value={retryModels[message.id] ?? ""} isDisabled={!online} onChange={(value) => setRetryModels((current) => ({ ...current, [message.id]: value }))} options={[{ value: "", label: "Same model" }, ...(modelQuery.data?.items.map((model) => ({ value: model.id, label: model.name })) ?? [])]} />
                </>}
                {siblings.length > 1 && <div className="ml-auto inline-flex items-center gap-[3px] font-mono text-[10px] text-slate-300" aria-label={`${user ? "User" : "Assistant"} branch`}>
                  <Button isIconOnly variant="ghost" className="h-7 w-7 text-lg leading-none text-slate-400" aria-label={`Previous branch for message ${message.id}`} isDisabled={!online || branchIndex <= 0 || switchMutation.isPending} onPress={() => switchMutation.mutate(siblings[branchIndex - 1].id)}>‹</Button>
                  <span>{branchIndex + 1} / {siblings.length}</span>
                  <Button isIconOnly variant="ghost" className="h-7 w-7 text-lg leading-none text-slate-400" aria-label={`Next branch for message ${message.id}`} isDisabled={!online || branchIndex >= siblings.length - 1 || switchMutation.isPending} onPress={() => switchMutation.mutate(siblings[branchIndex + 1].id)}>›</Button>
                </div>}
                {!user && original && hasFooterDetails(original) && <span className="ml-auto whitespace-nowrap font-mono text-[8px] text-slate-500" aria-label={usageLabel(original)}>{usageSummary(original)}</span>}
              </div>
            </MessagePrimitive.Root>;
          }}</ThreadPrimitive.Messages>
        </ThreadPrimitive.Viewport>
      </ThreadPrimitive.Root>
    </AssistantRuntimeProvider>}
    {streamingMessage && !streamingMessage.content && <div className="chat-thinking" role="status">{searchProgressLabel(streamingMessage)}</div>}

    <div className="mx-auto mt-auto w-full max-w-[710px] max-[720px]:mt-5">
      {draftError && <div className="mx-auto mb-2 max-w-[710px] rounded-lg border border-rose-400/20 bg-rose-400/[.04] px-2.5 py-2 text-[10px] text-rose-300" role="alert">{draftError}{draftError.toLowerCase().includes("api key") && <a className="ml-2 text-[var(--accent)]" href="/settings">Open Settings</a>}</div>}
      {fileError && <div className="mx-auto mb-2 flex max-w-[710px] items-center justify-between rounded-lg border border-rose-400/20 bg-rose-400/[.04] px-2.5 py-2 text-[10px] text-rose-300" role="alert">{fileError}<Button isIconOnly variant="ghost" className="h-6 w-6 text-rose-300" onPress={() => setFileError(undefined)} aria-label="Dismiss upload error"><X size={12} /></Button></div>}
      {!canSendImages && visibleMessages.some((message) => message.attachments.some((attachment) => attachment.kind === "image")) && <div className="mx-auto mb-2 max-w-[700px] text-[9px] text-amber-200" role="status">This model cannot read images attached earlier in this conversation.</div>}
      {hasOmittedScan && <div className="mx-auto mb-2 max-w-[700px] text-[9px] text-amber-200" role="status">A scanned PDF has no extractable text and will be omitted because this model cannot read PDF files.</div>}
      <AssistantRuntimeProvider runtime={runtime}>
        <ComposerPrimitive.Root className="relative w-full rounded-[15px] border border-[rgba(120,160,220,.17)] bg-[rgba(19,26,41,.82)] px-3.5 pt-3.5 pb-2.5 shadow-[0_10px_44px_rgba(61,232,255,.055),0_18px_60px_rgba(0,0,0,.17)] data-[disabled=true]:opacity-55 max-[720px]:px-[11px] max-[720px]:pt-[11px] max-[720px]:pb-2">
          <ComposerPrimitive.Input className="block min-h-[31px] max-h-[170px] w-full resize-none border-0 bg-transparent px-0.5 pb-2 text-[13px] text-[var(--text)] outline-none placeholder:text-slate-500" aria-label="Message" placeholder="Message Sprinter…" rows={1} submitMode={coarsePointer ? "ctrlEnter" : "enter"} disabled={!online || Boolean(streamingMessage) || send.isPending} />
          <UploadChips items={pendingUploads} onRemove={removeUpload} onRetry={retryPdfText} contextLength={activeModel?.context_length} modelName={activeModel?.name} canReadPdfs={canReadPdfs} />
          <div className="flex flex-wrap items-center justify-between gap-2"><div className="flex flex-wrap items-center gap-[7px]"><Button variant="secondary" onPress={() => fileInput.current?.click()} aria-label="Attach files" isDisabled={!online || Boolean(streamingMessage) || send.isPending}><Paperclip size={14} /> Attach</Button>
            {supportedTools.map((tool) => {
              const enabled = selectedTools.includes(tool.id);
              const incompatible = tool.id === WEB_SEARCH ? selectedTools.includes(BASH) : selectedTools.includes(WEB_SEARCH);
              const title = tool.id === WEB_SEARCH
                ? `Runs inside the selected provider under zero data retention routing. ~$0.01 per search.${incompatible ? " Can't be combined with Bash: search could run through a third-party engine." : ""}`
                : "Runs commands in an OpenRouter sandbox with no internet access. Files persist within this chat. ~$0.003 per session start.";
              const Icon = tool.id === WEB_SEARCH ? Globe : SquareTerminal;
              return <Tooltip key={tool.id} delay={350}>
                <Tooltip.Trigger><Button variant="ghost" className={`tool-pill ${enabled ? "tool-pill-on" : ""}`} aria-pressed={enabled} aria-label={`${toolLabel(tool.id)}${enabled ? " enabled" : " disabled"}`} isDisabled={!online || Boolean(streamingMessage) || send.isPending || savingTools} onPress={() => toggleTool(tool.id)}><Icon size={14} /><span className="max-[720px]:hidden">{toolLabel(tool.id)}</span></Button></Tooltip.Trigger>
                <Tooltip.Content className="z-50 max-w-[280px] rounded-lg border border-[var(--border)] bg-[var(--panel-solid)] px-2.5 py-2 text-[10px] leading-5 text-slate-200 shadow-xl">{title}</Tooltip.Content>
              </Tooltip>;
            })}
            {(activeModel?.upstream_tools.length ?? 0) > 0 && <Tooltip delay={350}>
              <Tooltip.Trigger><Button variant="ghost" className="tool-pill tool-pill-upstream" aria-disabled="true" aria-label={`Supported upstream but not by Sprinter: ${(activeModel?.upstream_tools ?? []).map(toolLabel).join(", ")}`}><Plus size={14} /></Button></Tooltip.Trigger>
              <Tooltip.Content className="z-50 max-w-[280px] rounded-lg border border-[var(--border)] bg-[var(--panel-solid)] px-2.5 py-2 text-[10px] leading-5 text-slate-200 shadow-xl">{(activeModel?.upstream_tools ?? []).map(toolLabel).join(", ")} are supported upstream but not by Sprinter</Tooltip.Content>
            </Tooltip>}
            {!chatId && <DefaultModelPicker isDisabled={!online || Boolean(streamingMessage)} />}</div><div className="flex items-center gap-[9px]"><span className="flex items-center gap-1 text-[9px] text-slate-500 max-[720px]:hidden">{online ? "Press enter to send" : "Reconnect to send"}</span>{streamingMessage ? <ComposerPrimitive.Cancel className="composer-action-danger" disabled={!online}><Square size={14} /> Stop</ComposerPrimitive.Cancel> : <ComposerPrimitive.Send className="composer-action-primary" aria-label="Send message" disabled={!online || pendingUploads.some((item) => item.uploading || Boolean(item.error))}><ArrowUpRight size={17} /></ComposerPrimitive.Send>}</div></div>
        </ComposerPrimitive.Root>
      </AssistantRuntimeProvider>
      <p className="mt-2 mb-0 text-center text-[9px] text-slate-600">Sprinter can make mistakes. Check important information.</p>
    </div>
  </section>;
}

function UploadChips({
  items,
  onRemove,
  onRetry,
  contextLength,
  modelName,
  canReadPdfs,
}: {
  items: PendingUpload[];
  onRemove?: (item: PendingUpload) => void;
  onRetry?: (item: PendingUpload) => void | Promise<void>;
  contextLength?: number;
  modelName?: string;
  canReadPdfs?: boolean;
}) {
  if (!items.length) return null;
  return <div className="flex flex-wrap gap-1.5 py-1" aria-label="Attachments">
    {items.map((item) => {
      const file = item.record;
      const id = file?.upload_id;
      const filename = file?.filename ?? item.filename ?? "Uploading file";
      const image = file?.kind === "image";
      const pdf = file?.kind === "pdf";
      const chars = file?.text_chars;
      const pages = file?.text_pages;
      const emptyPages = file?.text_empty_pages;
      const allEmpty = pdf && pages != null && pages > 0 && emptyPages === pages;
      const tokens = chars == null ? null : Math.ceil(chars / 4);
      const tooLarge = tokens != null && contextLength != null && tokens > contextLength;
      let status = item.error ?? (item.uploading
        ? item.pageProgress
          ? `Reading page ${item.pageProgress.page} of ${item.pageProgress.pages}…`
          : item.extracting ? "Reading PDF…" : `Uploading ${item.progress}%`
        : file
          ? pdf && allEmpty
            ? "No text found (scanned?)"
            : pdf && tokens != null
              ? `${formatBytes(file.size)} · ~${formatTokenEstimate(tokens)} tokens${emptyPages ? ` · ${emptyPages} of ${pages} pages have no text` : ""}`
              : `${formatBytes(file.size)} · ${file.kind}`
          : "Preparing upload…");
      if (!item.error && !item.uploading && pdf && tooLarge && tokens != null) {
        status = `${formatBytes(file.size)} · ~${formatTokenEstimate(tokens)} tokens${emptyPages ? ` · ${emptyPages} of ${pages} pages have no text` : ""}`;
      }
      const tooltip = allEmpty
        ? `No text could be extracted. This may be a scan. ${canReadPdfs ? "This model can read PDF files." : "This model cannot read PDF files."}`
        : emptyPages && pages
          ? `${emptyPages} of ${pages} pages have little or no text and may be scans. Scanned pages aren't read.`
          : tooLarge && contextLength != null
            ? `Larger than ${modelName ?? "the selected model"}'s ${formatTokenEstimate(contextLength)} context`
            : undefined;
      return <div className={`relative flex min-h-[38px] w-[min(220px,100%)] items-center gap-[7px] overflow-hidden rounded-lg border px-[7px] py-[5px] text-[#a9b5c8] ${item.error ? "border-rose-400/40" : "border-[rgba(120,160,220,.13)]"} bg-[rgba(7,12,22,.55)]`} key={item.key}>
        {image && id ? onRemove ? <img className="size-[30px] shrink-0 rounded object-cover" src={`/api/uploads/${encodeURIComponent(id)}`} alt="" /> : <a href={`/api/uploads/${encodeURIComponent(id)}`} target="_blank" rel="noreferrer" aria-label={`Open ${filename}`}><img className="size-[30px] shrink-0 rounded object-cover" src={`/api/uploads/${encodeURIComponent(id)}`} alt="" /></a> : <span className="grid size-7 shrink-0 place-items-center rounded bg-[var(--accent-soft)] text-[var(--accent)]">{file?.kind === "pdf" || file?.kind === "text" ? <FileText size={14} /> : <ImageIcon size={14} />}</span>}
        <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[9px] text-slate-200" title={filename}>{filename}<small className={`mt-0.5 block overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[8px] ${item.error ? "text-rose-300" : tooLarge ? "text-amber-200" : "text-slate-500"}`} title={tooltip}>{status}</small></span>
        {item.uploading && <span className="absolute bottom-0 left-0 h-0.5 bg-[var(--accent)] transition-[width] duration-150" style={{ width: `${Math.max(4, item.progress)}%` }} />}
        {onRetry && item.error && item.extractedText && <Button variant="ghost" className="h-6 min-w-0 px-1.5 text-[8px] text-[var(--accent)]" onPress={() => void onRetry(item)}>Retry</Button>}
        {onRemove && <Button isIconOnly variant="ghost" className="ml-auto h-5 w-5 shrink-0 rounded-md text-slate-500 hover:bg-rose-500/10 hover:text-rose-300" onPress={() => onRemove(item)} aria-label={`Remove ${filename}`}><X size={13} /></Button>}
        {id && !onRemove && !image && <a className="ml-auto text-[8px] text-[var(--accent)] no-underline" href={`/api/uploads/${encodeURIComponent(id)}`} target="_blank" rel="noreferrer" aria-label={`Open ${filename}`}>Open</a>}
      </div>;
    })}
  </div>;
}

function formatBytes(size: number) {
  if (size < 1024) return `${size} B`;
  if (size < 1024 * 1024) return `${(size / 1024).toFixed(0)} KB`;
  return `${(size / (1024 * 1024)).toFixed(1)} MB`;
}

function formatTokenEstimate(tokens: number) {
  return tokens >= 1000 ? `${Math.round(tokens / 1000)}k` : new Intl.NumberFormat().format(tokens);
}

function withPdfTextStats(record: UploadRecord, stats: PdfTextStats): UploadRecord {
  return {
    ...record,
    text: stats,
    text_chars: stats.chars,
    text_pages: stats.pages,
    text_empty_pages: stats.empty_pages,
  };
}

function errorName(error: unknown) {
  return error instanceof Error ? error.name : "UnknownError";
}

function ToolAwareMarkdown() {
  const part = useMessagePartText();
  const text = part.text;
  const message = useContext(ToolMessageContext);
  const steps = (message?.tool_steps ?? [])
    .filter((step) => step.tool === BASH)
    .slice()
    .sort((left, right) => left.offset - right.offset);
  if (!steps.length) return <MarkdownContent text={text} />;
  const chars = Array.from(text);
  const parts: ReactNode[] = [];
  let cursor = 0;
  for (const step of steps) {
    const offset = Math.max(cursor, Math.min(chars.length, step.offset));
    parts.push(<MarkdownContent key={`${step.id}-before`} text={chars.slice(cursor, offset).join("")} />);
    parts.push(<BashToolStep key={step.id} step={step} />);
    cursor = offset;
  }
  parts.push(<MarkdownContent key="tool-step-tail" text={chars.slice(cursor).join("")} />);
  return <>{parts}</>;
}

function BashToolStep({ step }: { step: ToolStep }) {
  const command = typeof step.input === "object" && step.input !== null && "command" in step.input && typeof step.input.command === "string"
    ? step.input.command
    : "Bash command";
  const exitCode = step.output?.exit_code;
  return <details className="bash-tool-step">
    <summary>
      <span className="bash-tool-step-label">Ran</span>
      <code title={command}>{command}</code>
      {step.status === "running" ? <span className="bash-tool-status" role="status"><span className="tool-spinner" aria-hidden="true" /> Running</span>
        : step.status === "error" ? <span className="bash-tool-status bash-tool-error">Stopped</span>
          : <span className={`bash-tool-exit ${exitCode === 0 ? "" : "bash-tool-error"}`}>exit {exitCode ?? "?"}</span>}
    </summary>
    <div className="bash-tool-output">
      {step.output?.stdout ? <pre aria-label="Command standard output">{step.output.stdout}</pre> : null}
      {step.output?.stderr ? <pre className="bash-tool-stderr" aria-label="Command error output">{step.output.stderr}</pre> : null}
      {!step.output && step.status === "running" && <span className="text-slate-400">Waiting for sandbox output…</span>}
      {!step.output && step.status === "error" && <span className="text-slate-400">No output was returned.</span>}
    </div>
  </details>;
}

function CitationChips({ citations }: { citations: Citation[] }) {
  if (!citations.length) return null;
  return <div className="citation-chips" aria-label="Web search sources">
    {citations.map((citation) => <a key={citation.url} href={citation.url} target="_blank" rel="noopener noreferrer" title={citation.title}>
      <span>{citationDomain(citation.url)}</span><strong>{citation.title}</strong>
    </a>)}
  </div>;
}

function citationDomain(value: string) {
  try { return new URL(value).hostname; } catch { return value; }
}

function searchProgressLabel(message?: ChatMessage) {
  const count = (message?.tool_steps ?? []).filter((step) => step.tool === WEB_SEARCH && step.status === "running").length;
  return count ? `SEARCHING… (${count})` : "THINKING…";
}

function hasFooterDetails(message: ChatMessage) {
  return message.prompt_tokens != null
    || message.completion_tokens != null
    || message.cost != null
    || message.tool_cost != null
    || message.web_search_requests != null
    || Boolean(message.tools?.length)
    || Boolean(message.tool_steps?.length);
}

function usageSummary(message: ChatMessage) {
  const tokenCount = message.prompt_tokens != null || message.completion_tokens != null
    ? new Intl.NumberFormat().format((message.prompt_tokens ?? 0) + (message.completion_tokens ?? 0))
    : null;
  const cost = message.cost == null ? null : `$${message.cost.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 6 })}`;
  const enabled = message.tools ?? [];
  const steps = message.tool_steps ?? [];
  const details: string[] = [];
  if (tokenCount) details.push(`${tokenCount} tokens`);
  if (cost) details.push(cost);
  if (enabled.includes(WEB_SEARCH)) {
    const count = message.web_search_requests ?? steps.filter((step) => step.tool === WEB_SEARCH).length;
    details.push(`Web search · ${count} ${count === 1 ? "search" : "searches"}`);
  }
  if (enabled.includes(BASH)) {
    const count = steps.filter((step) => step.tool === BASH).length;
    details.push(`Bash · ${count} ${count === 1 ? "command" : "commands"}`);
  }
  if (message.tool_cost != null) details.push(`$${message.tool_cost.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 6 })} tool fees`);
  return details.join(" · ");
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
