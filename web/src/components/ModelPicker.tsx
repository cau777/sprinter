import { useMemo } from "react";
import { Autocomplete, Button, ListBox, SearchField } from "@heroui/react";
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

export function ModelPicker({ models, value, favorites = [], onChange, onToggleFavorite, placeholder = "Choose a model", isDisabled, compact, className = "w-full" }: Props) {
  const items = useMemo(() => [...models].sort((a, b) => {
    const aFavorite = favorites.indexOf(a.id);
    const bFavorite = favorites.indexOf(b.id);
    if (aFavorite >= 0 || bFavorite >= 0) return (aFavorite < 0 ? Number.MAX_SAFE_INTEGER : aFavorite) - (bFavorite < 0 ? Number.MAX_SAFE_INTEGER : bFavorite);
    return a.name.localeCompare(b.name);
  }), [favorites, models]);
  const selected = models.find((model) => model.id === value);

  return (
    <Autocomplete<ApiModel, "single">
      aria-label="Model"
      className={className}
      items={items as unknown as Iterable<ApiModel, "single">}
      selectedKey={value}
      onSelectionChange={(key) => { if (typeof key === "string") onChange(key); }}
      isDisabled={isDisabled}
    >
      <Autocomplete.Trigger className={`flex w-full items-center justify-between gap-2 rounded-lg border border-[var(--field-border)] bg-[rgba(7,11,19,.48)] px-2.5 text-left text-[10px] text-slate-300 ${compact ? "min-h-8 rounded-full border-[var(--accent-line)] bg-[rgba(7,11,19,.38)] px-2.5" : "min-h-10"}`}>
        <Autocomplete.Value className={`flex min-w-0 items-center gap-2 overflow-hidden text-ellipsis whitespace-nowrap font-mono ${compact ? "text-[9px]" : "text-[10px]"}`}>{selected ? <><span className="size-1.5 shrink-0 rounded-full bg-slate-500" />{selected.id}</> : placeholder}</Autocomplete.Value>
        <Autocomplete.Indicator className="flex text-slate-400"><ChevronDown size={13} /></Autocomplete.Indicator>
      </Autocomplete.Trigger>
      <Autocomplete.Popover className="z-40 max-h-[min(420px,65vh)] w-[var(--trigger-width)] overflow-auto rounded-xl border border-[var(--border)] bg-[var(--panel-solid)] shadow-xl">
        <Autocomplete.Filter>
          <SearchField aria-label="Search models" className="sticky top-0 z-10 border-b border-[var(--border)] bg-[var(--panel-solid)] p-2">
            <SearchField.Group className="flex h-8 items-center gap-2 rounded-md border border-[var(--border)] px-2 text-slate-500"><SearchField.SearchIcon /><SearchField.Input className="w-full border-0 bg-transparent text-[10px] text-[var(--text)] outline-none placeholder:text-slate-500" placeholder="Search models…" /></SearchField.Group>
          </SearchField>
          <ListBox<ApiModel> className="p-1 outline-none" items={items}>
            {(model) => <ListBox.Item key={model.id} id={model.id} textValue={`${model.id} ${model.name}`} className="flex cursor-pointer items-center gap-2 rounded-lg px-2 py-2 text-[var(--text)] outline-none data-[focused]:bg-[var(--accent-soft)] data-[selected]:bg-[var(--accent-soft)]">
              <span className="grid min-w-0 flex-1 gap-0.5"><span className="text-[10px] font-semibold text-slate-200">{model.name}</span><span className="overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[9px] text-slate-500">{model.id}</span><span className="flex flex-wrap items-center gap-2 font-mono text-[8px] text-slate-500"><span>{(model.context_length / 1000).toFixed(0)}k context</span><span>In {moneyPerMillion(model.pricing.prompt)} / Out {moneyPerMillion(model.pricing.completion)} per 1M</span><span className="inline-flex items-center gap-1 text-cyan-300">{model.input_modalities.includes("image") ? <><Image size={11} /> image</> : null}{model.input_modalities.includes("file") || model.input_modalities.includes("pdf") ? <><FileText size={11} /> pdf</> : null}</span></span></span>
              {onToggleFavorite && <Button isIconOnly variant="ghost" className="size-7 rounded-md p-1 text-amber-300" aria-label={`${favorites.includes(model.id) ? "Remove" : "Add"} ${model.name} ${favorites.includes(model.id) ? "from" : "to"} favorites`} onPointerDown={(event) => event.stopPropagation()} onPress={() => onToggleFavorite(model.id)}><Star size={14} fill={favorites.includes(model.id) ? "currentColor" : "none"} /></Button>}
              {value === model.id && <Check className="shrink-0 text-[var(--accent)]" size={14} />}
            </ListBox.Item>}
          </ListBox>
        </Autocomplete.Filter>
      </Autocomplete.Popover>
    </Autocomplete>
  );
}
