import type { ReactNode } from "react";

export function SettingsGroup({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="settings-group">
      <h2>{title}</h2>
      <div>{children}</div>
    </section>
  );
}

export function SettingRow({
  label,
  sub,
  children,
}: {
  label: string;
  sub?: string;
  children: ReactNode;
}) {
  return (
    <label className="setting-row">
      <div>
        <strong>{label}</strong>
        {sub && <small>{sub}</small>}
      </div>
      {children}
    </label>
  );
}

export function Toggle({
  label,
  sub,
  checked,
  onChange,
}: {
  label: string;
  sub?: string;
  checked: boolean;
  onChange: (value: boolean) => void;
}) {
  return (
    <label className="setting-row">
      <div>
        <strong>{label}</strong>
        {sub && <small>{sub}</small>}
      </div>
      <input
        className="toggle"
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
    </label>
  );
}

export function Field({
  label,
  children,
  className,
}: {
  label: string;
  children: ReactNode;
  className?: string;
}) {
  return (
    <label className={`form-field ${className || ""}`.trim()}>
      <span>{label}</span>
      {children}
    </label>
  );
}
