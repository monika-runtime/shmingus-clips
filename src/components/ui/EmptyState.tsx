import type { ReactNode } from "react";

interface EmptyStateProps {
  icon: ReactNode;
  title: string;
  message: string;
  actions?: ReactNode;
  compact?: boolean;
}

export function EmptyState({
  icon,
  title,
  message,
  actions,
  compact,
}: EmptyStateProps) {
  return (
    <div className={compact ? "empty empty--compact" : "empty"}>
      <div className="empty__filmstrip" aria-hidden="true">
        <span className="empty__hole" />
        <span className="empty__frame">{icon}</span>
        <span className="empty__hole" />
      </div>
      <h3 className="empty__title">{title}</h3>
      <p className="empty__message">{message}</p>
      {actions && <div className="empty__actions">{actions}</div>}
    </div>
  );
}
