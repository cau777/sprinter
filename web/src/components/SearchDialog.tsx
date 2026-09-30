import { useEffect, useRef, useState } from "react";
import { Button, Input, Modal } from "@heroui/react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Search, X } from "lucide-react";
import { searchChats } from "../api/search";

type Props = { open: boolean; onClose: () => void };

export function SearchDialog({ open, onClose }: Props) {
  const input = useRef<HTMLInputElement>(null);
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");

  useEffect(() => {
    if (open) {
      setQuery("");
      window.setTimeout(() => input.current?.focus(), 0);
    }
  }, [open]);

  useEffect(() => {
    const timer = window.setTimeout(() => setDebounced(query.trim()), 180);
    return () => window.clearTimeout(timer);
  }, [query]);

  const results = useQuery({
    queryKey: ["search", debounced],
    queryFn: ({ signal }) => searchChats(debounced, signal),
    enabled: open && debounced.length > 0,
    staleTime: 0,
  });

  function openResult(chatId: string, messageId: string | null) {
    onClose();
    void navigate({ to: "/$chatId", params: { chatId }, search: { messageId: messageId ?? undefined } });
  }

  return <Modal isOpen={open} onOpenChange={(isOpen) => { if (!isOpen) onClose(); }}>
    <Modal.Backdrop className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm">
      <Modal.Container className="fixed inset-0 z-50 flex w-full items-center justify-center p-3 sm:w-full" placement="center">
        <Modal.Dialog aria-labelledby="chat-search-title" className="mx-auto flex max-h-[min(80dvh,680px)] w-[min(620px,calc(100vw-28px))] flex-col overflow-hidden rounded-2xl border border-[var(--border)] bg-[#0d1421] text-[var(--text)] shadow-2xl">
          <Modal.Header className="flex items-center justify-between px-5 pt-4 pb-3">
            <div><p className="mb-[11px] font-mono text-[11px] tracking-[.16em] text-[#8290a9]">YOUR CONVERSATIONS</p><Modal.Heading id="chat-search-title" className="m-0 font-display text-lg font-medium text-slate-100">Search chats</Modal.Heading></div>
            <Button isIconOnly variant="ghost" className="h-8 w-8 rounded-lg text-slate-400" aria-label="Close search" onPress={onClose}><X size={17} /></Button>
          </Modal.Header>
          <Modal.Body className="flex min-h-0 flex-1 flex-col px-5">
            <label className="flex h-11 shrink-0 items-center gap-2.5 rounded-lg border border-[var(--accent-line)] bg-[rgba(4,9,17,.68)] px-3 text-[var(--accent)]">
              <Search size={16} aria-hidden="true" />
              <Input ref={input} type="search" aria-label="Search messages and chat titles" className="min-w-0 flex-1 border-0 bg-transparent text-xs text-[var(--text)] outline-none placeholder:text-slate-500" placeholder="Search messages and titles…" value={query} onChange={(event) => setQuery(event.target.value)} />
              <kbd className="rounded border border-[var(--border)] px-1.5 py-0.5 font-mono text-[11px] text-slate-500">ESC</kbd>
            </label>
            <div className="min-h-24 flex-1 overflow-y-auto py-2.5" aria-live="polite">
        {!query.trim() ? <p className="my-0 px-2 py-5 text-center text-[11px] text-slate-500">Search across your messages and conversation titles.</p>
          : debounced !== query.trim() || results.isLoading ? <p className="my-0 px-2 py-5 text-center text-[11px] text-slate-500">Searching…</p>
          : results.isError ? <p className="my-0 px-2 py-5 text-center text-[11px] text-rose-300" role="alert">{results.error.message}</p>
          : results.data?.length ? results.data.map((result, index) => <Button variant="ghost" className="button-list-item flex h-auto min-h-[52px] w-full flex-col items-start gap-1 rounded-lg border-b border-[var(--border)] p-2.5 text-left text-[13px] hover:bg-[var(--accent-soft)]" key={`${result.chat_id}:${result.message_id ?? "title"}:${index}`} onPress={() => openResult(result.chat_id, result.message_id)}>
              <span className="text-[11px] font-semibold text-slate-200">{result.chat_title || "Untitled conversation"}</span>
              <span className="w-full overflow-hidden text-ellipsis whitespace-nowrap text-[11px] text-slate-400">{result.snippet}</span>
            </Button>)
          : <p className="my-0 px-2 py-5 text-center text-[11px] text-slate-500">No matching conversations.</p>}
            </div>
          </Modal.Body>
          <Modal.Footer className="flex justify-between gap-3 border-t border-[var(--border)] px-5 py-2.5 text-[11px] text-slate-500"><span>Message results jump to the match.</span><span><kbd>⌘ K</kbd> to open</span></Modal.Footer>
        </Modal.Dialog>
      </Modal.Container>
    </Modal.Backdrop>
  </Modal>;
}
