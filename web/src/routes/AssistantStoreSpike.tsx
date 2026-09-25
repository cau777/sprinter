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
    <div className="spike-page">
      <div className="spike-heading">
        <h1>Thread runtime lab</h1>
        <p>A local tree and fake stream exercising assistant-ui against an external store.</p>
        <div className="spike-callout"><WandSparkles size={13} style={{ verticalAlign: "-2px", marginRight: 6 }} />The conversation below lives in a parent-linked message tree. Try editing a user turn, regenerating a reply, attaching a file, and cancelling a stream.</div>
      </div>
      <AssistantRuntimeProvider runtime={runtime}>
        <ThreadPrimitive.Root className="spike-thread-root">
          <ThreadPrimitive.Viewport className="spike-thread" turnAnchor="bottom">
            <ThreadPrimitive.Messages>
              {({ message }) => {
                const original = tree.find((item) => item.id === message.id);
                const user = message.role === "user";
                return (
                  <MessagePrimitive.Root key={message.id} className="spike-message" data-role={message.role} data-running={!user && running ? "true" : "false"}>
                    <div className="spike-message-content"><MessagePrimitive.Parts /></div>
                    {original?.attachments?.map((file) => <div className="spike-attachment" key={file.id}><Paperclip size={12} style={{ verticalAlign: "-2px" }} /> {file.name} <span>· {file.type}</span></div>)}
                    {user && <div className="spike-message-tools"><ActionBarPrimitive.Root><ActionBarPrimitive.Edit>Edit message</ActionBarPrimitive.Edit></ActionBarPrimitive.Root></div>}
                    {!user && <div className="spike-message-tools"><ActionBarPrimitive.Root><ActionBarPrimitive.Reload><RotateCw size={11} style={{ verticalAlign: "-2px" }} /> Regenerate</ActionBarPrimitive.Reload></ActionBarPrimitive.Root><BranchControls /></div>}
                  </MessagePrimitive.Root>
                );
              }}
            </ThreadPrimitive.Messages>
          </ThreadPrimitive.Viewport>
          <ComposerPrimitive.Root className="spike-composer">
            <ComposerPrimitive.Input aria-label="Message the fake assistant" placeholder="Try a new message…" submitMode="ctrlEnter" />
            <div className="spike-composer-attachments"><ComposerPrimitive.Attachments>{({ attachment: file }) => <span key={file.id}><Paperclip size={11} /> {file.name}</span>}</ComposerPrimitive.Attachments></div>
            <ComposerPrimitive.AddAttachment className="spike-attach-control" aria-label="Add file"><Paperclip size={15} /></ComposerPrimitive.AddAttachment>
            {running ? <ComposerPrimitive.Cancel><Square size={14} /> Stop</ComposerPrimitive.Cancel> : <ComposerPrimitive.Send>Send demo</ComposerPrimitive.Send>}
          </ComposerPrimitive.Root>
          <div className="spike-state"><span>external messages: {tree.length} · visible path: {visiblePath.length}</span><span>source of truth: fake local tree</span></div>
        </ThreadPrimitive.Root>
      </AssistantRuntimeProvider>
    </div>
  );
}

function BranchControls() {
  return (
    <BranchPickerPrimitive.Root className="branch-picker" hideWhenSingleBranch>
      <BranchPickerPrimitive.Previous aria-label="Previous branch">‹</BranchPickerPrimitive.Previous>
      <span><BranchPickerPrimitive.Number /> / <BranchPickerPrimitive.Count /></span>
      <BranchPickerPrimitive.Next aria-label="Next branch">›</BranchPickerPrimitive.Next>
    </BranchPickerPrimitive.Root>
  );
}
