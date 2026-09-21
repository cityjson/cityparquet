import * as React from "react";

import { cn } from "@/lib/utils";

interface SelectProps extends React.SelectHTMLAttributes<HTMLSelectElement> {
  /** Use mono font for schema, module and identifier fields. */
  mono?: boolean;
}

/**
 * A native `<select>` carrying the same box as {@link Input}, so a form that
 * mixes the two lines up. Native rather than a listbox widget: the options
 * are short, flat lists of names, and the platform control already gives
 * keyboard and screen-reader behaviour a custom one would have to rebuild.
 */
export const Select = React.forwardRef<HTMLSelectElement, SelectProps>(
  ({ className, mono, children, ...props }, ref) => (
    <select
      ref={ref}
      className={cn(
        "flex h-9 w-full rounded-md border border-paper-300 bg-white px-3 py-2 text-[14px] text-ink-900 transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-lake-300 focus-visible:ring-offset-1 focus-visible:border-lake-500 disabled:cursor-not-allowed disabled:opacity-50",
        mono && "font-mono",
        className,
      )}
      {...props}
    >
      {children}
    </select>
  ),
);
Select.displayName = "Select";
