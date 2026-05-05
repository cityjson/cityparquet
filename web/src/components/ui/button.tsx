import { Slot } from "@radix-ui/react-slot";
import { cva, type VariantProps } from "class-variance-authority";
import * as React from "react";

import { cn } from "@/lib/utils";

/**
 * Buttons follow the CityLake design system:
 * - sharp 4 px corners
 * - press = `translateY(1px)`, no scale
 * - hover darkens by one Lake step
 * - focus = 2 px Lake-300 outline, offset 2 px
 */
const buttonVariants = cva(
  "inline-flex items-center justify-center gap-1.5 whitespace-nowrap rounded-md text-sm font-medium font-sans transition-colors duration-150 ease-cl active:translate-y-[1px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-lake-300 focus-visible:ring-offset-2 focus-visible:ring-offset-paper-50 disabled:opacity-50 disabled:cursor-not-allowed",
  {
    variants: {
      variant: {
        primary:
          "bg-lake-500 text-paper-50 hover:bg-lake-700",
        secondary:
          "bg-white text-ink-900 border border-paper-300 hover:bg-paper-100",
        ghost:
          "bg-transparent text-lake-500 hover:bg-lake-50 hover:text-lake-700",
        danger:
          "bg-[hsl(var(--destructive))] text-[hsl(var(--destructive-foreground))] hover:bg-roof-700",
        link:
          "text-lake-500 hover:text-lake-700 underline-offset-4 hover:underline",
      },
      size: {
        sm: "h-7 px-2.5 text-xs",
        default: "h-9 px-4",
        lg: "h-10 px-5",
        icon: "h-8 w-8",
      },
    },
    defaultVariants: { variant: "primary", size: "default" },
  },
);

export interface ButtonProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement>,
    VariantProps<typeof buttonVariants> {
  asChild?: boolean;
}

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant, size, asChild = false, ...props }, ref) => {
    const Comp = asChild ? Slot : "button";
    return (
      <Comp
        className={cn(buttonVariants({ variant, size }), className)}
        ref={ref}
        {...props}
      />
    );
  },
);
Button.displayName = "Button";

export { buttonVariants };
