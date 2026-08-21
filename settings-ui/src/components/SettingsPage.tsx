import type { ReactNode } from "react";

interface SettingsPageProps {
  title: string;
  description: string;
  children: ReactNode;
}

export function SettingsPage({
  title,
  description,
  children,
}: SettingsPageProps) {
  return (
    <div className="settings-page">
      <header className="page-heading">
        <h1>{title}</h1>
        <p>{description}</p>
      </header>
      <div className="page-content">{children}</div>
    </div>
  );
}
