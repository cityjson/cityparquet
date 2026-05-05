import { cn } from "@/lib/utils";

type Tone = "ok" | "warn" | "error" | "info" | "neutral";

const toneClasses: Record<Tone, string> = {
  ok: "bg-moss-500",
  warn: "bg-sun-500",
  error: "bg-[hsl(var(--destructive))]",
  info: "bg-lake-500",
  neutral: "bg-ink-400",
};

/**
 * 6 px coloured dot used inside tags and table-status indicators
 * (e.g. `● READY`, `● COMPACTING`).
 */
export function StatusDot({ tone = "neutral" }: { tone?: Tone }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "inline-block h-1.5 w-1.5 rounded-full shrink-0",
        toneClasses[tone],
      )}
    />
  );
}
