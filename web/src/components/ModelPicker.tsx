import { useId, useMemo, useState } from "react";
import { Button, Input, Modal } from "@heroui/react";
import { BadgeDollarSign, CalendarDays, Check, ChevronDown, FileText, Image, Search, Star, X } from "lucide-react";
import type { ApiModel } from "../api/settings";

type Props = {
  models: ApiModel[];
  value: string | null;
  favorites?: string[];
  onChange: (modelId: string) => void;
  onToggleFavorite?: (modelId: string) => void;
  placeholder?: string;
  isDisabled?: boolean;
  compact?: boolean;
  className?: string;
};

const moneyPerMillion = (value: string) => {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return "—";
  return parsed === 0 ? "Free" : `$${(parsed * 1_000_000).toFixed(parsed < 0.00001 ? 2 : 3)}`;
};

export function filterModels(models: ApiModel[], query: string) {
  const normalized = query.trim().toLocaleLowerCase();
  if (!normalized) return models;
  return models.filter((model) => `${model.name} ${model.id}`.toLocaleLowerCase().includes(normalized));
}

export function isRecentlyReleased(model: ApiModel, now = Date.now()) {
  if (model.created_at == null) return false;
  const cutoff = new Date(now);
  cutoff.setUTCMonth(cutoff.getUTCMonth() - 6);
  // OpenRouter uses Unix seconds; tolerate millisecond timestamps too. The
  // provider and device clocks can differ, so only apply the six-month floor.
  const releasedAt = model.created_at < 100_000_000_000 ? model.created_at * 1000 : model.created_at;
  return releasedAt >= cutoff.getTime();
}

export function isCheapModel(model: ApiModel) {
  const inputPrice = Number(model.pricing.prompt);
  return Number.isFinite(inputPrice) && inputPrice >= 0 && inputPrice < 0.5 / 1_000_000;
}

export function filterModelsByQuickFilters(
  models: ApiModel[],
  { newModelsOnly, cheapOnly }: { newModelsOnly: boolean; cheapOnly: boolean },
  now = Date.now(),
) {
  return models.filter((model) =>
    (!newModelsOnly || isRecentlyReleased(model, now)) && (!cheapOnly || isCheapModel(model)),
  );
}

export function ModelPicker({ models, value, favorites = [], onChange, onToggleFavorite, placeholder = "Choose a model", isDisabled, compact, className = "w-full" }: Props) {
  const titleId = useId();
  const [query, setQuery] = useState("");
  const [isOpen, setIsOpen] = useState(false);
  const [newModelsOnly, setNewModelsOnly] = useState(true);
  const [cheapOnly, setCheapOnly] = useState(false);
  const selected = models.find((model) => model.id === value);
  const items = useMemo(() => {
    const ordered = [...models].sort((a, b) => {
      const aFavorite = favorites.indexOf(a.id);
      const bFavorite = favorites.indexOf(b.id);
      if (aFavorite >= 0 || bFavorite >= 0) return (aFavorite < 0 ? Number.MAX_SAFE_INTEGER : aFavorite) - (bFavorite < 0 ? Number.MAX_SAFE_INTEGER : bFavorite);
      return a.name.localeCompare(b.name);
    });
    return filterModelsByQuickFilters(filterModels(ordered, query), { newModelsOnly, cheapOnly });
  }, [cheapOnly, favorites, models, newModelsOnly, query]);

  return <>
    <Button
      variant={compact ? "ghost" : "secondary"}
      className={`${className} flex min-w-0 items-center justify-between gap-2 ${compact ? "min-h-8 rounded-full border border-[var(--accent-line)] bg-[rgba(7,11,19,.38)] px-2.5 text-[9px]" : "min-h-10 rounded-lg px-2.5 text-[10px]"}`}
      aria-label={selected ? `Choose model, current ${selected.id}` : placeholder}
      aria-haspopup="dialog"
      isDisabled={isDisabled}
      onPress={() => setIsOpen(true)}
    >
      <span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap font-mono text-slate-300">{selected?.id ?? placeholder}</span>
      <ChevronDown size={13} className="shrink-0 text-slate-400" />
    </Button>

    <Modal isOpen={isOpen} onOpenChange={(open) => { setIsOpen(open); if (!open) setQuery(""); }}>
      <Modal.Backdrop className="fixed inset-0 z-50 bg-black/70 backdrop-blur-sm">
        <Modal.Container className="fixed inset-0 z-50 flex w-full items-center justify-center p-3" placement="center" size={"lg"}>
          <Modal.Dialog aria-labelledby={titleId} className="mx-auto flex max-h-[min(86dvh,780px)] w-[min(820px,calc(100vw-28px))] flex-col overflow-hidden rounded-2xl border border-[var(--border)] bg-[#0d1421] text-[var(--text)] shadow-2xl">
            <Modal.Header className="flex items-center justify-between border-b border-[var(--border)] px-5 py-4">
              <div>
                <p className="eyebrow text-center">MODEL LIBRARY</p>
                <Modal.Heading id={titleId} className="m-0 font-display text-lg font-medium text-slate-100">Choose a model</Modal.Heading>
              </div>
              <Button isIconOnly variant="ghost" className="h-8 w-8 rounded-lg text-slate-400" aria-label="Close model picker" onPress={() => setIsOpen(false)}><X size={17} /></Button>
            </Modal.Header>

            <Modal.Body className="flex min-h-0 flex-1 flex-col gap-3 p-0 pt-3">
              <label className="flex h-10 shrink-0 items-center gap-2.5 rounded-lg border border-[var(--accent-line)] bg-[rgba(4,9,17,.68)] px-3 text-[var(--accent)]">
                <Search size={15} aria-hidden="true" />
                <Input aria-label="Search models" className="min-w-0 flex-1 border-0 bg-transparent text-xs text-[var(--text)] outline-none placeholder:text-slate-500" placeholder="Search model names and IDs…" value={query} onChange={(event) => setQuery(event.target.value)} />
              </label>

              <div className="flex shrink-0 flex-wrap items-center gap-2" aria-label="Quick filters">
                <Button variant={newModelsOnly ? "secondary" : "ghost"} className={`gap-1.5 rounded-full px-3 py-1.5 text-[10px] ${newModelsOnly ? "border border-[var(--accent-line)] text-[var(--accent)]" : "border border-[var(--border)] text-slate-400"}`} aria-pressed={newModelsOnly} onPress={() => setNewModelsOnly((enabled) => !enabled)}>
                  <CalendarDays size={13} /> New models <span className="text-[9px] opacity-70">6 months</span>
                </Button>
                <Button variant={cheapOnly ? "secondary" : "ghost"} className={`gap-1.5 rounded-full px-3 py-1.5 text-[10px] ${cheapOnly ? "border border-[var(--accent-line)] text-[var(--accent)]" : "border border-[var(--border)] text-slate-400"}`} aria-pressed={cheapOnly} onPress={() => setCheapOnly((enabled) => !enabled)}>
                  <BadgeDollarSign size={13} /> Cheap models <span className="text-[9px] opacity-70">under $0.50 / 1M input</span>
                </Button>
                <span className="ml-auto font-mono text-[9px] text-slate-500">{items.length} models</span>
              </div>

              <ul aria-label="Available models" className="m-0 min-h-0 flex-1 list-none overflow-y-auto rounded-xl border border-[var(--border)] p-1">
                {items.map((model) => {
                  const isFavorite = favorites.includes(model.id);
                  const isSelected = value === model.id;
                  return <li key={model.id} className={`flex items-center gap-2 rounded-lg border-b border-[var(--border)] px-2 py-1 last:border-0 ${isSelected ? "bg-[var(--accent-soft)]" : "hover:bg-white/[.025]"}`}>
                    <button type="button" className="flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-2 text-left outline-none transition-colors focus-visible:ring-2 focus-visible:ring-[var(--accent)]" aria-pressed={isSelected} onClick={() => { onChange(model.id); setIsOpen(false); setQuery(""); }}>
                      <span className="grid min-w-0 flex-1 gap-1 text-left">
                        <span className="flex min-w-0 items-center gap-2 text-[11px] font-semibold text-slate-100"><span className="truncate">{model.name}</span>{isSelected && <span className="shrink-0 text-[9px] font-normal text-[var(--accent)]">Current model</span>}</span>
                        <span className="truncate font-mono text-[9px] text-slate-500">{model.id}</span>
                        <span className="flex flex-wrap items-center gap-x-3 gap-y-1 font-mono text-[9px] leading-4 text-slate-400">
                          <span>{(model.context_length / 1000).toFixed(0)}k context</span>
                          <span>In {moneyPerMillion(model.pricing.prompt)} / Out {moneyPerMillion(model.pricing.completion)} per 1M</span>
                          <span className="inline-flex items-center gap-1 text-cyan-300">{model.input_modalities.includes("image") && <><Image size={11} /> image</>}{(model.input_modalities.includes("file") || model.input_modalities.includes("pdf")) && <><FileText size={11} /> pdf</>}</span>
                        </span>
                      </span>
                      {isSelected && <Check className="shrink-0 text-[var(--accent)]" size={15} />}
                    </button>
                    {onToggleFavorite && <Button isIconOnly variant="ghost" className="size-8 shrink-0 rounded-md text-amber-300" aria-label={`${isFavorite ? "Remove" : "Add"} ${model.name} ${isFavorite ? "from" : "to"} favorites`} onPress={() => onToggleFavorite(model.id)}><Star size={15} fill={isFavorite ? "currentColor" : "none"} /></Button>}
                  </li>;
                })}
                {items.length === 0 && <li className="px-3 py-10 text-center text-xs text-slate-500">No models match these filters.</li>}
              </ul>
            </Modal.Body>
          </Modal.Dialog>
        </Modal.Container>
      </Modal.Backdrop>
    </Modal>
  </>;
}
