import type { ReactNode } from "react";

import { cx } from "../../lib/cx";

interface FieldProps {
  label: string;
  hint?: ReactNode;
  className?: string;
  children: ReactNode;
}

export function Field({ label, hint, className, children }: FieldProps) {
  return (
    <div className={cx("field", className)}>
      <span className="field__label">{label}</span>
      {children}
      {hint && <div className="field__hint">{hint}</div>}
    </div>
  );
}
