import type { ReactNode } from "react";

import { cx } from "../../lib/cx";

type BadgeTone = "accent" | "warn" | "danger" | "info" | "neutral";

interface BadgeProps {
  tone?: BadgeTone;
  dot?: boolean;
  pulse?: boolean;
  children: ReactNode;
  className?: string;
}

export function Badge({
  tone = "neutral",
  dot,
  pulse,
  children,
  className,
}: BadgeProps) {
  return (
    <span className={cx("badge", `badge--${tone}`, className)}>
      {dot && <span className={cx("badge__dot", pulse && "is-pulsing")} aria-hidden="true" />}
      {children}
    </span>
  );
}

interface ChipProps {
  icon?: ReactNode;
  mono?: boolean;
  title?: string;
  children: ReactNode;
  className?: string;
}

export function Chip({ icon, mono, title, children, className }: ChipProps) {
  return (
    <span className={cx("chip", mono && "chip--mono", className)} title={title}>
      {icon && <span className="chip__icon">{icon}</span>}
      <span className="chip__text">{children}</span>
    </span>
  );
}
