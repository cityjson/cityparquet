import * as React from "react";

import { cn } from "@/lib/utils";

interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  /** Use mono font for SQL/path/ID fields. */
  mono?: boolean;
}

export const Input = React.forwardRef<HTMLInputElement, InputProps>(
  ({ className, mono, type, ...props }, ref) => (
    <input
      type={type}
      ref={ref}
      className={cn(
        "flex h-9 w-full rounded-md border border-paper-300 bg-white px-3 py-2 text-[14px] text-ink-900 transition-colors placeholder:text-ink-400 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-lake-300 focus-visible:ring-offset-1 focus-visible:border-lake-500 disabled:cursor-not-allowed disabled:opacity-50",
        mono && "font-mono",
        className,
      )}
      {...props}
    />
  ),
);
Input.displayName = "Input";
