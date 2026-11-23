import { AlertTriangle, CheckCircle2, Info } from "lucide-react";
import type { ReactNode } from "react";

import { cx } from "../../lib/cx";

interface AlertProps {
  tone?: "danger" | "warn" | "info" | "success";
  title?: string;
  children: ReactNode;
  action?: ReactNode;
  className?: string;
}

const ICONS = {
  danger: <AlertTriangle size={17} />,
  warn: <AlertTriangle size={17} />,
  info: <Info size={17} />,
  success: <CheckCircle2 size={17} />,
};

export function Alert({ tone = "danger", title, children, action, className }: AlertProps) {
  return (
    <div className={cx("alert", `alert--${tone}`, className)} role="note">
      <span className="alert__icon" aria-hidden="true">
        {ICONS[tone]}
      </span>
      <div className="alert__body">
        {title && <p className="alert__title">{title}</p>}
        <div className="alert__text">{children}</div>
      </div>
      {action && <div className="alert__action">{action}</div>}
    </div>
  );
}
