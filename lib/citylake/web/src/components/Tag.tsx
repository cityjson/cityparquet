import * as React from "react";

import { cn } from "@/lib/utils";

type Tone = "neutral" | "ok" | "warn" | "error" | "info";

interface Props extends React.HTMLAttributes<HTMLSpanElement> {
  tone?: Tone;
  /** Square corners instead of pill — used for codes like `EPSG:7415`. */
  square?: boolean;
}

const toneClasses: Record<Tone, string> = {
  ok: "bg-moss-100 text-moss-700",
  warn: "bg-sun-100 text-sun-700",
  error: "bg-roof-100 text-roof-700",
  info: "bg-lake-100 text-lake-900",
  neutral: "bg-paper-100 text-ink-700 border border-paper-300",
};

/**
 * Mono-font tag pill (or square). Used for status badges (`READY`,
 * `COMPACTING`) and identifier codes (`EPSG:7415`, LOD selectors).
 */
export const Tag = React.forwardRef<HTMLSpanElement, Props>(
  ({ tone = "neutral", square = false, className, ...props }, ref) => (
    <span
      ref={ref}
      className={cn(
        "inline-flex items-center gap-1.5 font-mono text-[11px] px-2 py-[3px]",
        square ? "rounded-sm" : "rounded-full",
        toneClasses[tone],
        className,
      )}
      {...props}
    />
  ),
);
Tag.displayName = "Tag";
