import { useMemo } from "react";
import { Autocomplete, ListBox, SearchField } from "@heroui/react";
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
};

const moneyPerMillion = (value: string) => {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return "—";
  return parsed === 0 ? "Free" : `$${(parsed * 1_000_000).toFixed(parsed < 0.00001 ? 2 : 3)}`;
};

export function ModelPicker({ models, value, favorites = [], onChange, onToggleFavorite, placeholder = "Choose a model", isDisabled }: Props) {
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
      className="model-autocomplete"
      items={items as unknown as Iterable<ApiModel, "single">}
      selectedKey={value}
      onSelectionChange={(key) => { if (typeof key === "string") onChange(key); }}
      isDisabled={isDisabled}
    >
      <Autocomplete.Trigger className="model-trigger">
        <Autocomplete.Value>{selected ? <><span className="model-indicator" />{selected.id}</> : placeholder}</Autocomplete.Value>
        <Autocomplete.Indicator><ChevronDown size={13} /></Autocomplete.Indicator>
      </Autocomplete.Trigger>
      <Autocomplete.Popover className="model-popover">
        <Autocomplete.Filter>
          <SearchField aria-label="Search models" className="model-search">
            <SearchField.Group><SearchField.SearchIcon /><SearchField.Input placeholder="Search models…" /></SearchField.Group>
          </SearchField>
          <ListBox<ApiModel> className="model-list" items={items}>
            {(model) => <ListBox.Item key={model.id} id={model.id} textValue={`${model.id} ${model.name}`} className="model-option">
              <span className="model-option-copy"><span className="model-option-title">{model.name}</span><span className="model-option-id">{model.id}</span><span className="model-option-meta"><span>{(model.context_length / 1000).toFixed(0)}k context</span><span>In {moneyPerMillion(model.pricing.prompt)} / Out {moneyPerMillion(model.pricing.completion)} per 1M</span><span className="model-modalities">{model.input_modalities.includes("image") ? <><Image size={11} /> image</> : null}{model.input_modalities.includes("file") || model.input_modalities.includes("pdf") ? <><FileText size={11} /> pdf</> : null}</span></span></span>
              {onToggleFavorite && <button className="model-star" type="button" aria-label={`${favorites.includes(model.id) ? "Remove" : "Add"} ${model.name} ${favorites.includes(model.id) ? "from" : "to"} favorites`} onClick={(event) => { event.preventDefault(); event.stopPropagation(); onToggleFavorite(model.id); }}><Star size={14} fill={favorites.includes(model.id) ? "currentColor" : "none"} /></button>}
              {value === model.id && <Check className="model-selected-icon" size={14} />}
            </ListBox.Item>}
          </ListBox>
        </Autocomplete.Filter>
      </Autocomplete.Popover>
    </Autocomplete>
  );
}
