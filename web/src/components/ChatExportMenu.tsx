type Props = { chatId: string };

export function ChatExportMenu({ chatId }: Props) {
  const base = `/api/chats/${encodeURIComponent(chatId)}/export`;
  return <details className="chat-export-menu">
    <summary aria-label="Export conversation">Export</summary>
    <div className="chat-export-options">
      <a href={`${base}?format=md`}>Markdown · visible branch</a>
      <a href={`${base}?format=json`}>JSON · full message tree</a>
    </div>
  </details>;
}
