import { createContext, useContext, type ReactNode } from "react";
import { matchesSearchQuery } from "../settings/settings-fusion";

export const SettingsSearchContext = createContext("");

export function SettingsGroup({
  title,
  badge,
  anchorId,
  span = "auto",
  flashing = false,
  className,
  children,
}: {
  title: string;
  badge?: string;
  anchorId?: string;
  span?: "full" | "auto";
  flashing?: boolean;
  className?: string;
  children: ReactNode;
}) {
  const classes = [
    "settings-group",
    span === "full" ? "settings-group-full" : "",
    flashing ? "settings-anchor-flash" : "",
    className || "",
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <section className={classes} data-settings-anchor={anchorId}>
      <div className="settings-group-header">
        <h2>{title}</h2>
        {badge && <span className="settings-group-badge">{badge}</span>}
      </div>
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
  const query = useContext(SettingsSearchContext);
  const hit = matchesSearchQuery(query, label, sub);
  return (
    <label className={hit ? "setting-row setting-row-hit" : "setting-row"}>
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
  const query = useContext(SettingsSearchContext);
  const hit = matchesSearchQuery(query, label, sub);
  return (
    <label className={hit ? "setting-row setting-row-hit" : "setting-row"}>
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
