import type { ReactNode } from "react";

interface PageHeaderProps {
  eyebrow: string;
  title: string;
  description: string;
  actions?: ReactNode;
}

export function PageHeader({ eyebrow, title, description, actions }: PageHeaderProps) {
  return (
    <header className="page-head">
      <div className="page-head__text">
        <p className="eyebrow">{eyebrow}</p>
        <h1 className="page-head__title">{title}</h1>
        <p className="page-head__description">{description}</p>
      </div>
      {actions && <div className="page-head__actions">{actions}</div>}
    </header>
  );
}
