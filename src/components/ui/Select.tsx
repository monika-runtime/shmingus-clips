import { ChevronDown } from "lucide-react";

import { cx } from "../../lib/cx";

interface SelectProps {
  value: string;
  options: Array<{ value: string; label: string }>;
  onChange: (value: string) => void;
  label: string;
  disabled?: boolean;
  className?: string;
}

export function Select({
  value,
  options,
  onChange,
  label,
  disabled,
  className,
}: SelectProps) {
  return (
    <span className={cx("select", disabled && "is-disabled", className)}>
      <select
        className="select__control"
        value={value}
        aria-label={label}
        disabled={disabled}
        onChange={(event) => onChange(event.target.value)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
      <ChevronDown size={16} className="select__chevron" aria-hidden="true" />
    </span>
  );
}
