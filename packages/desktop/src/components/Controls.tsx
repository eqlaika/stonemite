import {
  useId,
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type ReactNode,
  type SelectHTMLAttributes,
  type TextareaHTMLAttributes,
} from "react";

import type { OptionItem } from "../settings/types";

export function FormSection({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: ReactNode;
}) {
  return (
    <section className="form-section">
      <div className="section-heading">
        <h2>{title}</h2>
        {description ? <p>{description}</p> : null}
      </div>
      <div className="section-fields">{children}</div>
    </section>
  );
}

export function Field({
  label,
  description,
  htmlFor,
  descriptionId,
  children,
}: {
  label: string;
  description?: string;
  htmlFor?: string;
  descriptionId?: string;
  children: ReactNode;
}) {
  return (
    <div className="field-row">
      <div className="field-copy">
        <label htmlFor={htmlFor}>{label}</label>
        {description ? <p id={descriptionId}>{description}</p> : null}
      </div>
      <div className="field-control">{children}</div>
    </div>
  );
}

export function TextInput(props: InputHTMLAttributes<HTMLInputElement>) {
  return <input className="text-input" {...props} />;
}

export function TextArea(props: TextareaHTMLAttributes<HTMLTextAreaElement>) {
  return <textarea className="text-area" {...props} />;
}

export function RangeInput({
  value,
  min,
  max,
  step,
  suffix,
  onChange,
  ariaLabel,
  ariaDescribedBy,
}: {
  value: number;
  min: number;
  max: number;
  step?: number;
  suffix?: string;
  onChange: (value: number) => void;
  ariaLabel: string;
  ariaDescribedBy?: string;
}) {
  return (
    <div className="range-control">
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        aria-label={ariaLabel}
        aria-describedby={ariaDescribedBy}
        onChange={(event) => onChange(Number(event.currentTarget.value))}
      />
      <output>{`${value}${suffix ?? ""}`}</output>
    </div>
  );
}

export function CheckboxOption({
  label,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  const id = useId();
  return (
    <label className="checkbox-option" htmlFor={id}>
      <input
        id={id}
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
      <span>{label}</span>
    </label>
  );
}

export function SelectInput<T extends string>({
  options,
  ...props
}: SelectHTMLAttributes<HTMLSelectElement> & { options: OptionItem<T>[] }) {
  return (
    <select className="select-input" {...props}>
      {options.map((option) => (
        <option key={option.value} value={option.value}>
          {option.label}
        </option>
      ))}
    </select>
  );
}

export function Toggle({
  label,
  description,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  description?: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
  disabled?: boolean;
}) {
  const id = useId();
  const descriptionId = description ? `${id}-description` : undefined;
  return (
    <div className="toggle-row">
      <div className="field-copy">
        <label htmlFor={id}>{label}</label>
        {description ? <p id={descriptionId}>{description}</p> : null}
      </div>
      <input
        id={id}
        className="toggle-input"
        aria-describedby={descriptionId}
        type="checkbox"
        role="switch"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.checked)}
      />
    </div>
  );
}

type ButtonVariant = "primary" | "secondary" | "quiet" | "danger";

export function Button({
  variant = "secondary",
  className = "",
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: ButtonVariant }) {
  return (
    <button
      className={`button button-${variant} ${className}`.trim()}
      {...props}
    />
  );
}

export function InlineStatus({
  tone,
  title,
  children,
}: {
  tone: "info" | "success" | "warning" | "error";
  title: string;
  children?: ReactNode;
}) {
  return (
    <div
      className={`inline-status status-${tone}`}
      role={tone === "error" ? "alert" : "status"}
    >
      <strong>{title}</strong>
      {children ? <div>{children}</div> : null}
    </div>
  );
}
