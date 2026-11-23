import type { ReactNode } from "react";

import { cx } from "../../lib/cx";

interface SwitchProps {
  checked: boolean;
  onChange: (checked: boolean) => void;
  label: string;
  description?: string;
  icon?: ReactNode;
  disabled?: boolean;
  compact?: boolean;
  className?: string;
}

export function Switch({
  checked,
  onChange,
  label,
  description,
  icon,
  disabled,
  compact,
  className,
}: SwitchProps) {
  return (
    <div
      className={cx(
        "switch-row",
        compact && "switch-row--compact",
        disabled && "is-disabled",
        className,
      )}
    >
      {icon && <span className="switch-row__icon">{icon}</span>}
      <span className="switch-row__text">
        <span className="switch-row__label">{label}</span>
        {description && <span className="switch-row__desc">{description}</span>}
      </span>
      <button
        type="button"
        role="switch"
        aria-checked={checked}
        aria-label={label}
        disabled={disabled}
        className={cx("switch", checked && "is-on")}
        onClick={() => onChange(!checked)}
      >
        <span className="switch__thumb" aria-hidden="true" />
      </button>
    </div>
  );
}
