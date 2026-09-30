import { useMemo, useRef, useState } from "react";
import type { ThreadMessageLike } from "@assistant-ui/react";
import {
  AssistantRuntimeProvider,
  ActionBarPrimitive,
  BranchPickerPrimitive,
  CompositeAttachmentAdapter,
  SimpleImageAttachmentAdapter,
  SimpleTextAttachmentAdapter,
  ComposerPrimitive,
  MessagePrimitive,
  ThreadPrimitive,
  useExternalStoreRuntime,
} from "@assistant-ui/react";
import type { AppendMessage } from "@assistant-ui/react";
import { Paperclip, RotateCw, Square, WandSparkles } from "lucide-react";

type DemoMessage = ThreadMessageLike & { id: string; parentId: string | null; attachmentName?: string };

const starter: DemoMessage[] = [
  { id: "u-1", role: "user", parentId: null, content: "Help me plan a small weekend garden. I have a photo of the space, too.", attachmentName: "garden-corner.jpg" },
  { id: "a-1", role: "assistant", parentId: "u-1", content: "A great place to start. For a compact garden, think in layers: one taller focal plant, herbs within reach, and a few low flowers to soften the edges.\n\nI can help tailor a layout once I know how much direct sun the space gets." },
];

function messageText(message: AppendMessage) {
  return message.content.filter((part) => part.type === "text").map((part) => part.text).join("");
}

export function AssistantStoreSpike() {
  const [tree, setTree] = useState<DemoMessage[]>(starter);
  const [leafId, setLeafId] = useState("a-1");
  const [running, setRunning] = useState(false);
  const nextId = useRef(2);
  const abort = useRef<AbortController | null>(null);

  const visiblePath = useMemo(() => {
    const path: DemoMessage[] = [];
    let cursor = tree.find((item) => item.id === leafId);
    while (cursor) {
      path.unshift(cursor);
      cursor = cursor.parentId ? tree.find((item) => item.id === cursor?.parentId) : undefined;
    }
    return path;
  }, [tree, leafId]);

  const addTurn = (text: string, parentId: string | null, attachments?: AppendMessage["attachments"]) => {
    const userId = `u-${nextId.current++}`;
    const assistantId = `a-${nextId.current++}`;
    const user: DemoMessage = { id: userId, parentId, role: "user", content: text, ...(attachments?.length ? { attachments } : {}) };
    const answer = "That gives us a useful starting point. I’d keep the first version simple: map the light, pick a few dependable plants, and leave room to adjust after the first week.\n\nIf you tell me the sun exposure, I can suggest a specific layout.";
    const assistant: DemoMessage = { id: assistantId, parentId: userId, role: "assistant", content: "" };
    setTree((current) => [...current, user, assistant]);
    setLeafId(assistantId);
    setRunning(true);
    const controller = new AbortController();
    abort.current = controller;
    let pos = 0;
    const tick = () => {
      if (controller.signal.aborted) return;
      pos = Math.min(answer.length, pos + 4);
      setTree((current) => current.map((item) => item.id === assistantId ? { ...item, content: answer.slice(0, pos) } : item));
      if (pos === answer.length) {
        setRunning(false);
        abort.current = null;
      } else window.setTimeout(tick, 30);
    };
    window.setTimeout(tick, 280);
  };

  const addReply = (parentId: string | null) => {
    if (!parentId) return;
    const assistantId = `a-${nextId.current++}`;
    const answer = "Here’s another take: start with a narrow path and a small cluster of herbs near the door. It keeps the space useful while you learn how the light moves.";
    setTree((current) => [...current, { id: assistantId, parentId, role: "assistant", content: "" }]);
    setLeafId(assistantId);
    setRunning(true);
    const controller = new AbortController();
    abort.current = controller;
    let pos = 0;
    const tick = () => {
      if (controller.signal.aborted) return;
      pos = Math.min(answer.length, pos + 4);
      setTree((current) => current.map((item) => item.id === assistantId ? { ...item, content: answer.slice(0, pos) } : item));
      if (pos === answer.length) { setRunning(false); abort.current = null; }
      else window.setTimeout(tick, 30);
    };
    window.setTimeout(tick, 280);
  };

  const runtime = useExternalStoreRuntime({
    messages: visiblePath,
    isRunning: running,
    convertMessage: (message) => ({ ...message, content: message.content }),
    setMessages: (next) => {
      // The external store exposes one root-to-leaf path at a time; old nodes stay in the tree.
      const head = next.at(-1)?.id;
      if (head && tree.some((item) => item.id === head)) setLeafId(head);
    },
    unstable_onBranchChange: ({ headId }) => {
      // In the real adapter this maps to POST /api/chats/:id/switch.
      if (headId && tree.some((item) => item.id === headId)) setLeafId(headId);
    },
    adapters: { attachments: new CompositeAttachmentAdapter([new SimpleImageAttachmentAdapter(), new SimpleTextAttachmentAdapter()]) },
    onNew: async (message) => {
      addTurn(messageText(message), message.parentId, message.attachments);
    },
    onEdit: async (message) => addTurn(messageText(message), message.parentId, message.attachments),
    onReload: async (parentId) => addReply(parentId),
    onCancel: async () => {
      abort.current?.abort();
      abort.current = null;
      setRunning(false);
    },
  });

  return (
    <div className="mx-auto flex min-h-0 w-full max-w-[880px] flex-1 flex-col px-5 pt-[25px] pb-[15px] max-[720px]:px-[13px] max-[720px]:pt-[18px] max-[720px]:pb-[calc(13px+env(safe-area-inset-bottom))]">
      <div className="mb-[15px]">
        <h1 className="m-0 font-display text-xl font-medium text-[#edf2fb]">Thread runtime lab</h1>
        <p className="mt-1 mb-1 text-[11px] text-[#8995aa]">A local tree and fake stream exercising assistant-ui against an external store.</p>
        <div className="my-2 rounded-[9px] border border-[rgba(61,232,255,.14)] bg-[rgba(61,232,255,.035)] px-[11px] py-[9px] text-[11px] text-[#96a4b9]"><WandSparkles size={13} style={{ verticalAlign: "-2px", marginRight: 6 }} />The conversation below lives in a parent-linked message tree. Try editing a user turn, regenerating a reply, attaching a file, and cancelling a stream.</div>
      </div>
      <AssistantRuntimeProvider runtime={runtime}>
        <ThreadPrimitive.Root className="flex min-h-0 flex-1 flex-col">
          <ThreadPrimitive.Viewport className="min-h-[220px] flex-1 overflow-auto py-4" turnAnchor="bottom">
            <ThreadPrimitive.Messages>
              {({ message }) => {
                const original = tree.find((item) => item.id === message.id);
                const user = message.role === "user";
                return (
                  <MessagePrimitive.Root key={message.id} className="mx-auto mb-[19px] w-full max-w-[700px] text-[#d1d9e6] [&_p]:m-0 [&_p]:whitespace-pre-wrap data-[role=user]:ml-auto data-[role=user]:max-w-[560px] data-[role=user]:rounded-[14px_14px_4px_14px] data-[role=user]:border data-[role=user]:border-[var(--accent-line)] data-[role=user]:bg-[var(--surface)] data-[role=user]:px-3.5 data-[role=user]:py-3 data-[role=user]:text-[#e2e8f2] data-[role=assistant]:border-l-2 data-[role=assistant]:border-l-[var(--accent)] data-[role=assistant]:py-0.5 data-[role=assistant]:pl-3.5 data-[role=assistant][data-running=true]:shadow-[-4px_0_20px_-12px_var(--accent)]" data-role={message.role} data-running={!user && running ? "true" : "false"}>
                    {original?.attachments?.map((file) => <div className="mt-1.5 font-mono text-[11px] text-[var(--accent)]" key={file.id}><Paperclip size={12} style={{ verticalAlign: "-2px" }} /> {file.name} <span>· {file.type}</span></div>)}
                    {user && <div className="mt-[9px] flex items-center gap-1 [&_button]:min-h-8 [&_button]:rounded-[7px] [&_button]:border [&_button]:border-[rgba(120,160,220,.2)] [&_button]:bg-[#182236] [&_button]:px-2 [&_button]:text-xs [&_button]:text-[#d7deea] [&_button]:cursor-pointer [&_span]:text-xs [&_span]:text-[#b0bdcf]"><ActionBarPrimitive.Root><ActionBarPrimitive.Edit>Edit message</ActionBarPrimitive.Edit></ActionBarPrimitive.Root></div>}
                    {!user && <div className="mt-[9px] flex items-center gap-1 [&_button]:min-h-8 [&_button]:rounded-[7px] [&_button]:border [&_button]:border-[rgba(120,160,220,.2)] [&_button]:bg-[#182236] [&_button]:px-2 [&_button]:text-xs [&_button]:text-[#d7deea] [&_button]:cursor-pointer [&_span]:text-xs [&_span]:text-[#b0bdcf]"><ActionBarPrimitive.Root><ActionBarPrimitive.Reload><RotateCw size={11} style={{ verticalAlign: "-2px" }} /> Regenerate</ActionBarPrimitive.Reload></ActionBarPrimitive.Root><BranchControls /></div>}
                  </MessagePrimitive.Root>
                );
              }}
            </ThreadPrimitive.Messages>
          </ThreadPrimitive.Viewport>
          <ComposerPrimitive.Root className="flex gap-[7px] rounded-[13px] border border-[var(--border)] bg-[rgba(19,26,41,.82)] p-[9px]">
            <ComposerPrimitive.Input className="min-h-[43px] max-h-[120px] flex-1 resize-y border-0 bg-transparent p-[5px] text-[var(--text)] outline-none" aria-label="Message the fake assistant" placeholder="Try a new message…" submitMode="ctrlEnter" />
            <div className="flex flex-wrap gap-1"><ComposerPrimitive.Attachments>{({ attachment: file }) => <span className="flex max-w-[120px] items-center gap-1 overflow-hidden rounded-[5px] bg-[var(--accent-soft)] px-[5px] py-[3px] font-mono text-[11px] text-[var(--accent)]" key={file.id}><Paperclip size={11} /> {file.name}</span>}</ComposerPrimitive.Attachments></div>
            <ComposerPrimitive.AddAttachment className="grid h-[35px] min-w-[34px] place-items-center rounded-lg border border-[rgba(120,160,220,.2)] bg-[#182236] p-0 text-[#d7deea]" aria-label="Add file"><Paperclip size={15} /></ComposerPrimitive.AddAttachment>
            {running ? <ComposerPrimitive.Cancel className="inline-flex min-h-10 items-center justify-center gap-2 rounded-[10px] border border-transparent px-3.5 font-sans text-[13px] font-semibold leading-[1.2] text-[#f8fafc] bg-[rgba(255,93,122,.13)] border-[rgba(255,93,122,.25)] hover:bg-[rgba(255,93,122,.2)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-50"><Square size={14} /> Stop</ComposerPrimitive.Cancel> : <ComposerPrimitive.Send className="inline-flex min-h-10 w-auto items-center justify-center gap-2 rounded-[10px] border border-transparent bg-[var(--accent)] px-3.5 font-sans text-[13px] font-semibold leading-[1.2] text-[#061018] transition-colors hover:bg-[#6cecff] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--accent)] disabled:cursor-not-allowed disabled:opacity-50">Send demo</ComposerPrimitive.Send>}
          </ComposerPrimitive.Root>
          <div className="mt-[9px] flex justify-between gap-[15px] font-mono text-[11px] text-[#6e7a90]"><span>external messages: {tree.length} · visible path: {visiblePath.length}</span><span>source of truth: fake local tree</span></div>
        </ThreadPrimitive.Root>
      </AssistantRuntimeProvider>
    </div>
  );
}

function BranchControls() {
  return (
    <BranchPickerPrimitive.Root className="flex items-center" hideWhenSingleBranch>
      <BranchPickerPrimitive.Previous className="grid size-8 place-items-center rounded-lg text-xl leading-none text-[#b0bdcf] hover:bg-white/[.06] hover:text-[#f1f5f9]" aria-label="Previous branch">‹</BranchPickerPrimitive.Previous>
      <span><BranchPickerPrimitive.Number /> / <BranchPickerPrimitive.Count /></span>
      <BranchPickerPrimitive.Next className="grid size-8 place-items-center rounded-lg text-xl leading-none text-[#b0bdcf] hover:bg-white/[.06] hover:text-[#f1f5f9]" aria-label="Next branch">›</BranchPickerPrimitive.Next>
    </BranchPickerPrimitive.Root>
  );
}
