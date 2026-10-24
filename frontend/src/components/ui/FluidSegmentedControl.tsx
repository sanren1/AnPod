import { useId, type CSSProperties, type ReactNode } from "react";
import { motion, useReducedMotion } from "motion/react";
import "../../design/components.css";

export type SegmentOption<T extends string> = {
  value: T;
  label: ReactNode;
  disabled?: boolean;
};

export function FluidSegmentedControl<T extends string>({
  label,
  value,
  options,
  onValueChange,
  columns = options.length,
  className,
}: {
  label: string;
  value: T;
  options: SegmentOption<T>[];
  onValueChange: (value: T) => void;
  columns?: number;
  className?: string;
}) {
  const id = useId().replaceAll(":", "");
  const reduceMotion = useReducedMotion();

  return <div
    className={`fluid-segmented-control${className ? ` ${className}` : ""}`}
    role="group"
    aria-label={label}
    style={{ "--segment-count": columns } as CSSProperties}
  >
    {options.map(option => {
      const selected = option.value === value;
      return <button
        key={option.value}
        type="button"
        aria-pressed={selected}
        disabled={option.disabled}
        onClick={() => onValueChange(option.value)}
      >
        {selected && <motion.span
          className="fluid-segmented-control__selection"
          layoutId={`fluid-segment-${id}`}
          initial={false}
          transition={reduceMotion ? { duration: 0 } : { type: "spring", stiffness: 460, damping: 36 }}
          aria-hidden="true"
        />}
        <span>{option.label}</span>
      </button>;
    })}
  </div>;
}
