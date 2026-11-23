import type { InputHTMLAttributes, ReactNode } from "react";

import { cx } from "../../lib/cx";

interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  icon?: ReactNode;
  invalid?: boolean;
}

export function Input({ icon, invalid, className, ...rest }: InputProps) {
  if (!icon) {
    return (
      <input
        className={cx("input", invalid && "is-invalid", className)}
        {...rest}
      />
    );
  }
  return (
    <span className={cx("input-wrap", invalid && "is-invalid", className)}>
      <span className="input-wrap__icon" aria-hidden="true">
        {icon}
      </span>
      <input className="input input--with-icon" {...rest} />
    </span>
  );
}
