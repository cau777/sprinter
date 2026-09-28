import { useMemo, useRef, useState } from "react";
import { Autocomplete, Button, ListBox } from "@heroui/react";
import { Check, ChevronDown, FileText, Image, Star } from "lucide-react";
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

export function ModelPicker({ models, value, favorites = [], onChange, onToggleFavorite, placeholder = "Choose a model", isDisabled, compact, className = "w-full" }: Props) {
  const [query, setQuery] = useState("");
  const [isOpen, setIsOpen] = useState(false);
  const favoritePress = useRef(false);
  function keepPickerOpen(event: { stopPropagation: () => void }) {
    event.stopPropagation();
    favoritePress.current = true;
    window.setTimeout(() => { favoritePress.current = false; }, 0);
  }
  const items = useMemo(() => [...models].sort((a, b) => {
    const aFavorite = favorites.indexOf(a.id);
    const bFavorite = favorites.indexOf(b.id);
    if (aFavorite >= 0 || bFavorite >= 0) return (aFavorite < 0 ? Number.MAX_SAFE_INTEGER : aFavorite) - (bFavorite < 0 ? Number.MAX_SAFE_INTEGER : bFavorite);
    return a.name.localeCompare(b.name);
  }), [favorites, models]);
  const filteredItems = useMemo(() => filterModels(items, query), [items, query]);
  const selected = models.find((model) => model.id === value);

  return (
    <Autocomplete<ApiModel, "single">
      aria-label="Model"
      className={className}
      items={items as unknown as Iterable<ApiModel, "single">}
      selectedKey={value}
      onSelectionChange={(key) => { if (typeof key === "string") { onChange(key); setQuery(""); } }}
      isOpen={isOpen}
      onOpenChange={(open) => {
        if (!open && favoritePress.current) return;
        setIsOpen(open);
        if (!open) setQuery("");
      }}
      isDisabled={isDisabled}
    >
      <Autocomplete.Trigger className={`flex w-full items-center justify-between gap-2 rounded-lg border border-[var(--field-border)] bg-[rgba(7,11,19,.48)] px-2.5 text-left text-[10px] text-slate-300 ${compact ? "min-h-8 rounded-full border-[var(--accent-line)] bg-[rgba(7,11,19,.38)] px-2.5" : "min-h-10"}`}>
        <Autocomplete.Value className={`flex min-w-0 items-center gap-2 overflow-hidden text-ellipsis whitespace-nowrap font-mono ${compact ? "text-[9px]" : "text-[10px]"}`}>{selected ? <><span className="size-1.5 shrink-0 rounded-full bg-slate-500" />{selected.id}</> : placeholder}</Autocomplete.Value>
        <Autocomplete.Indicator className="flex text-slate-400"><ChevronDown size={13} /></Autocomplete.Indicator>
      </Autocomplete.Trigger>
      <Autocomplete.Popover className="z-40 max-h-[min(420px,65vh)] w-[var(--trigger-width)] overflow-auto rounded-xl border border-[var(--border)] bg-[var(--panel-solid)] shadow-xl">
        <div>
          <div className="sticky top-0 z-10 border-b border-[var(--border)] bg-[var(--panel-solid)] p-2">
            <input aria-label="Search models" className="h-9 w-full rounded-md border border-[var(--border)] bg-transparent px-2 text-xs text-[var(--text)] outline-none placeholder:text-slate-500 focus:border-[var(--accent-line)]" placeholder="Search models…" value={query} onChange={(event) => setQuery(event.target.value)} />
          </div>
          <ListBox<ApiModel> key={favorites.join("\u0000")} className="p-1 outline-none" items={filteredItems}>
            {(model) => {
              const isFavorite = favorites.includes(model.id);
              return <ListBox.Item key={model.id} id={model.id} textValue={`${model.id} ${model.name}`} className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-2 text-[var(--text)] outline-none data-[focused]:bg-[var(--accent-soft)] data-[selected]:bg-[var(--accent-soft)]">
                <span className="grid min-w-0 flex-1 gap-0.5"><span className="text-[10px] font-semibold text-slate-200">{model.name}</span><span className="overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[9px] text-slate-500">{model.id}</span><span className="flex flex-wrap items-center gap-2 font-mono text-[8px] text-slate-500"><span>{(model.context_length / 1000).toFixed(0)}k context</span><span>In {moneyPerMillion(model.pricing.prompt)} / Out {moneyPerMillion(model.pricing.completion)} per 1M</span><span className="inline-flex items-center gap-1 text-cyan-300">{model.input_modalities.includes("image") ? <><Image size={11} /> image</> : null}{model.input_modalities.includes("file") || model.input_modalities.includes("pdf") ? <><FileText size={11} /> pdf</> : null}</span></span></span>
                {onToggleFavorite && <Button isIconOnly variant="ghost" className="size-7 rounded-md p-1 text-amber-300" aria-label={`${isFavorite ? "Remove" : "Add"} ${model.name} ${isFavorite ? "from" : "to"} favorites`} onPointerDownCapture={keepPickerOpen} onPointerUpCapture={keepPickerOpen} onClickCapture={(event) => {
                  keepPickerOpen(event);
                  onToggleFavorite(model.id);
                  setIsOpen(true);
                }}><Star size={14} fill={isFavorite ? "currentColor" : "none"} /></Button>}
                {value === model.id && <Check className="shrink-0 text-[var(--accent)]" size={14} />}
              </ListBox.Item>;
            }}
          </ListBox>
        </div>
      </Autocomplete.Popover>
    </Autocomplete>
  );
}
