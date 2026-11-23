import { cx } from "../../lib/cx";

interface SegmentedOption<T extends string | number> {
  value: T;
  label: string;
}

interface SegmentedProps<T extends string | number> {
  value: T;
  options: Array<SegmentedOption<T>>;
  onChange: (value: T) => void;
  label: string;
  disabled?: boolean;
  className?: string;
}

export function Segmented<T extends string | number>({
  value,
  options,
  onChange,
  label,
  disabled,
  className,
}: SegmentedProps<T>) {
  return (
    <div
      className={cx("segmented", disabled && "is-disabled", className)}
      role="group"
      aria-label={label}
    >
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={String(option.value)}
            type="button"
            className={cx("segmented__option", active && "is-active")}
            aria-pressed={active}
            disabled={disabled}
            onClick={() => onChange(option.value)}
          >
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
