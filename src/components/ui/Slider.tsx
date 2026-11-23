import type { CSSProperties } from "react";

import { clamp } from "../../lib/format";

interface SliderProps {
  value: number;
  min: number;
  max: number;
  step?: number;
  label: string;
  formatValue: (value: number) => string;
  onChange: (value: number) => void;
  disabled?: boolean;
  hint?: string;
}

export function Slider({
  value,
  min,
  max,
  step = 1,
  label,
  formatValue,
  onChange,
  disabled,
  hint,
}: SliderProps) {
  const percent = max > min ? clamp(((value - min) / (max - min)) * 100, 0, 100) : 0;
  return (
    <div className={disabled ? "slider-field is-disabled" : "slider-field"}>
      <div className="slider-field__head">
        <span className="micro-label">{label}</span>
        <span className="slider-field__value">{formatValue(value)}</span>
      </div>
      <input
        type="range"
        className="slider"
        min={min}
        max={max}
        step={step}
        value={value}
        disabled={disabled}
        aria-label={label}
        aria-valuetext={formatValue(value)}
        onChange={(event) => onChange(Number(event.target.value))}
        style={
          {
            "--slider-fill": `${percent}%`,
          } as CSSProperties
        }
      />
      {hint && <p className="field__hint">{hint}</p>}
    </div>
  );
}
