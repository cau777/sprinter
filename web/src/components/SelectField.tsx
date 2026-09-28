import { ChevronDown } from "lucide-react";
import { ListBox, Select } from "@heroui/react";

type Option = { value: string; label: string };

type Props = {
  "aria-label": string;
  value: string;
  options: Option[];
  onChange: (value: string) => void;
  isDisabled?: boolean;
  className?: string;
};

export function SelectField({ "aria-label": ariaLabel, value, options, onChange, isDisabled, className }: Props) {
  return (
    <Select<Option, "single">
      aria-label={ariaLabel}
      className={className}
      items={options as unknown as Iterable<Option, "single">}
      selectedKey={value}
      onSelectionChange={(key) => { if (typeof key === "string") onChange(key); }}
      isDisabled={isDisabled}
    >
      <Select.Trigger className="flex min-h-9 w-full items-center justify-between gap-2 rounded-lg border border-[var(--field-border)] bg-[rgba(7,11,19,.46)] px-2.5 text-left text-xs text-[var(--text)] outline-none focus-visible:border-[var(--accent-line)]">
        <Select.Value>{({ selectedText }) => selectedText ?? options.find((option) => option.value === value)?.label}</Select.Value>
        <Select.Indicator><ChevronDown size={14} /></Select.Indicator>
      </Select.Trigger>
      <Select.Popover className="z-50 min-w-[var(--trigger-width)] overflow-hidden rounded-lg border border-[var(--border)] bg-[var(--panel-solid)] p-1 shadow-xl">
        <ListBox<Option> className="max-h-64 overflow-y-auto outline-none">
          {options.map((option) => (
            <ListBox.Item
              key={option.value}
              id={option.value}
              textValue={option.label}
              className="cursor-pointer rounded-md px-2.5 py-2 text-xs text-[var(--text)] outline-none data-[focused]:bg-[var(--accent-soft)] data-[selected]:text-[var(--accent)]"
            >
              {option.label}
            </ListBox.Item>
          ))}
        </ListBox>
      </Select.Popover>
    </Select>
  );
}
