import * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Card primitive. Renders the design's drafting-paper card: white background,
 * 1 px paper-300 hairline, 6 px radius (8 px max), the soft `shadow-cl-1`
 * paper drop, and 18 px padding by default.
 *
 * Use the `accent` prop to flag a card with a 2 px coloured left border for
 * status conveying (success / warn / error). Don't add accent borders for
 * decoration — the design system reserves them for semantic meaning.
 */
type Accent = "ok" | "warn" | "error" | "info" | "primary";

const accentClasses: Record<Accent, string> = {
  ok: "border-l-2 border-l-moss-500",
  warn: "border-l-2 border-l-sun-500",
  error: "border-l-2 border-l-[hsl(var(--destructive))]",
  info: "border-l-2 border-l-lake-500",
  primary: "border-l-2 border-l-roof-500",
};

interface CardProps extends React.HTMLAttributes<HTMLDivElement> {
  accent?: Accent;
}

export const Card = React.forwardRef<HTMLDivElement, CardProps>(
  ({ className, accent, ...props }, ref) => (
    <div
      ref={ref}
      className={cn(
        "rounded-md border border-paper-300 bg-white text-ink-800 shadow-cl-1",
        accent && accentClasses[accent],
        className,
      )}
      {...props}
    />
  ),
);
Card.displayName = "Card";

export const CardHeader = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div ref={ref} className={cn("flex flex-col space-y-1 px-5 pt-5 pb-3", className)} {...props} />
  ),
);
CardHeader.displayName = "CardHeader";

export const CardTitle = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div
      ref={ref}
      className={cn(
        "font-sans text-[17px] font-semibold leading-snug tracking-tight text-ink-900",
        className,
      )}
      {...props}
    />
  ),
);
CardTitle.displayName = "CardTitle";

export const CardDescription = React.forwardRef<
  HTMLDivElement,
  React.HTMLAttributes<HTMLDivElement>
>(({ className, ...props }, ref) => (
  <div ref={ref} className={cn("text-[13px] text-ink-500 leading-normal", className)} {...props} />
));
CardDescription.displayName = "CardDescription";

export const CardContent = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div ref={ref} className={cn("px-5 pb-5", className)} {...props} />
  ),
);
CardContent.displayName = "CardContent";

export const CardFooter = React.forwardRef<HTMLDivElement, React.HTMLAttributes<HTMLDivElement>>(
  ({ className, ...props }, ref) => (
    <div ref={ref} className={cn("flex items-center px-5 pb-5", className)} {...props} />
  ),
);
CardFooter.displayName = "CardFooter";
