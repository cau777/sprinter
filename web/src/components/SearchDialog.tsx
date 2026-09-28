import { useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Search, X } from "lucide-react";
import { searchChats } from "../api/search";

type Props = { open: boolean; onClose: () => void };

export function SearchDialog({ open, onClose }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const navigate = useNavigate();
  const [query, setQuery] = useState("");
  const [debounced, setDebounced] = useState("");

  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) {
      element.showModal();
      window.setTimeout(() => input.current?.focus(), 0);
    } else if (!open && element.open) {
      element.close();
    }
    if (open) setQuery("");
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

  function openResult(chatId: string) {
    onClose();
    void navigate({ to: "/$chatId", params: { chatId } });
  }

  return <dialog
    ref={dialog}
    className="chat-search-dialog"
    aria-labelledby="chat-search-title"
    onCancel={(event) => { event.preventDefault(); onClose(); }}
    onClick={(event) => { if (event.target === dialog.current) onClose(); }}
  >
    <div className="chat-search-panel">
      <div className="chat-search-heading">
        <div><p className="eyebrow">YOUR CONVERSATIONS</p><h2 id="chat-search-title">Search chats</h2></div>
        <button type="button" className="chat-search-close" aria-label="Close search" onClick={onClose}><X size={17} /></button>
      </div>
      <label className="chat-search-input-wrap">
        <Search size={16} aria-hidden="true" />
        <input ref={input} type="search" aria-label="Search messages and chat titles" placeholder="Search messages and titles…" value={query} onChange={(event) => setQuery(event.target.value)} />
        <kbd>ESC</kbd>
      </label>
      <div className="chat-search-results" aria-live="polite">
        {!query.trim() ? <p className="chat-search-hint">Search across your messages and conversation titles.</p>
          : debounced !== query.trim() || results.isLoading ? <p className="chat-search-hint">Searching…</p>
          : results.isError ? <p className="chat-search-error" role="alert">{results.error.message}</p>
          : results.data?.length ? results.data.map((result, index) => <button type="button" className="chat-search-result" key={`${result.chat_id}:${result.message_id ?? "title"}:${index}`} onClick={() => openResult(result.chat_id)}>
              <span className="chat-search-result-title">{result.chat_title || "Untitled conversation"}</span>
              <span className="chat-search-result-snippet">{result.snippet}</span>
            </button>)
          : <p className="chat-search-hint">No matching conversations.</p>}
      </div>
      <div className="chat-search-footer"><span>Results open the matching conversation.</span><span><kbd>⌘ K</kbd> to open</span></div>
    </div>
  </dialog>;
}
