import { Button, Modal } from "@heroui/react";
import { CircleHelp, X } from "lucide-react";

type Props = { open: boolean; onClose: () => void };

const shortcuts = [
  { action: "Search conversations", keys: "⌘ K" },
  { action: "Start a new chat", keys: "⌘ ⇧ O" },
];

export function HelpDialog({ open, onClose }: Props) {
  return <Modal isOpen={open} onOpenChange={(isOpen) => { if (!isOpen) onClose(); }}>
    <Modal.Backdrop className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm">
      <Modal.Container className="fixed inset-0 z-50 flex w-full items-center justify-center p-3 sm:w-full" placement="center">
        <Modal.Dialog aria-labelledby="help-shortcuts-title" className="mx-auto w-[min(460px,calc(100vw-28px))] overflow-hidden rounded-2xl border border-[var(--border)] bg-[#0d1421] text-[var(--text)] shadow-2xl">
          <Modal.Header className="flex items-center justify-between px-5 pt-4 pb-3">
            <div>
              <p className="mb-[11px] font-mono text-[11px] tracking-[.16em] text-[#8290a9]">SPRINTER</p>
              <Modal.Heading id="help-shortcuts-title" className="m-0 font-display text-lg font-medium text-slate-100">Help & shortcuts</Modal.Heading>
            </div>
            <Button isIconOnly variant="ghost" className="h-8 w-8 rounded-lg text-slate-400" aria-label="Close help" onPress={onClose}><X size={17} /></Button>
          </Modal.Header>
          <Modal.Body className="px-5 pb-5">
            <div className="mb-3 flex items-start gap-3 rounded-xl border border-[var(--border)] bg-white/[.025] p-3 text-sm leading-relaxed text-slate-300">
              <CircleHelp className="mt-0.5 shrink-0 text-[var(--accent)]" size={17} />
              <p className="m-0">Use keyboard shortcuts to move around Sprinter. On Windows and Linux, use Ctrl in place of ⌘.</p>
            </div>
            <div className="divide-y divide-[var(--border)]">
              {shortcuts.map(({ action, keys }) => <div key={action} className="flex min-h-12 items-center justify-between gap-4 py-2 text-sm text-slate-300">
                <span>{action}</span><kbd className="shrink-0 rounded-md border border-[var(--border)] bg-white/[.035] px-2 py-1 font-mono text-xs text-slate-200">{keys}</kbd>
              </div>)}
            </div>
          </Modal.Body>
        </Modal.Dialog>
      </Modal.Container>
    </Modal.Backdrop>
  </Modal>;
}
