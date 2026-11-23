import type { ButtonHTMLAttributes, ReactNode } from "react";

import { cx } from "../../lib/cx";

type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: "sm" | "md" | "lg";
  icon?: ReactNode;
  trailing?: ReactNode;
  block?: boolean;
}

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  trailing,
  block,
  className,
  children,
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={cx(
        "btn",
        `btn--${variant}`,
        `btn--${size}`,
        block && "btn--block",
        className,
      )}
      {...rest}
    >
      {icon && <span className="btn__icon">{icon}</span>}
      {children !== undefined && <span className="btn__label">{children}</span>}
      {trailing && <span className="btn__trailing">{trailing}</span>}
    </button>
  );
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  variant?: "ghost" | "quiet" | "danger";
  size?: "sm" | "md";
}

export function IconButton({
  label,
  variant = "quiet",
  size = "md",
  className,
  children,
  type = "button",
  ...rest
}: IconButtonProps) {
  return (
    <button
      type={type}
      aria-label={label}
      title={label}
      className={cx("icon-btn", `icon-btn--${variant}`, `icon-btn--${size}`, className)}
      {...rest}
    >
      {children}
    </button>
  );
}
