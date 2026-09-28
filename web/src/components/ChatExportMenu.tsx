import { Button, Dropdown } from "@heroui/react";

type Props = { chatId: string };

export function ChatExportMenu({ chatId }: Props) {
  const base = `/api/chats/${encodeURIComponent(chatId)}/export`;
  return <Dropdown>
    <Dropdown.Trigger>
      <Button variant="outline" className="rounded-lg border-[var(--border)] px-2.5 py-1.5 text-[10px] text-slate-400 hover:border-[var(--accent-line)] hover:text-[var(--accent)]">Export</Button>
    </Dropdown.Trigger>
    <Dropdown.Popover className="z-50 min-w-48 rounded-lg border border-[var(--border)] bg-[var(--panel-solid)] p-1 shadow-xl">
      <Dropdown.Menu aria-label="Export conversation" className="outline-none">
        <Dropdown.Item id="markdown" href={`${base}?format=md`} className="cursor-pointer rounded-md px-2 py-2 text-xs text-slate-300 outline-none data-[focused]:bg-[var(--accent-soft)] data-[focused]:text-[var(--accent)]">Markdown · visible branch</Dropdown.Item>
        <Dropdown.Item id="json" href={`${base}?format=json`} className="cursor-pointer rounded-md px-2 py-2 text-xs text-slate-300 outline-none data-[focused]:bg-[var(--accent-soft)] data-[focused]:text-[var(--accent)]">JSON · full message tree</Dropdown.Item>
      </Dropdown.Menu>
    </Dropdown.Popover>
  </Dropdown>;
}
