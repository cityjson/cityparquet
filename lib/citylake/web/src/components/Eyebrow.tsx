import * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Mono-font, uppercased, wide-tracked label used throughout the CityLake design
 * system as a section eyebrow (e.g. `TABLES · 5`, `CITYJSON METADATA`).
 */
export const Eyebrow = React.forwardRef<HTMLSpanElement, React.HTMLAttributes<HTMLSpanElement>>(
  ({ className, ...props }, ref) => (
    <span
      ref={ref}
      className={cn(
        "font-mono text-[11px] tracking-caps uppercase text-ink-500 leading-tight",
        className,
      )}
      {...props}
    />
  ),
);
Eyebrow.displayName = "Eyebrow";
