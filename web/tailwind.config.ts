import type { Config } from "tailwindcss";
import animate from "tailwindcss-animate";

/**
 * Tailwind theme aligned with the CityLake design system.
 *
 * - Colour tokens map to the CSS variables in `src/styles/citylake.css` (Lake,
 *   Ink, Paper, Roof, Moss, Sun) plus the shadcn-style semantic aliases.
 * - Border radii top out at 8px — sharp corners are part of the brand.
 * - The default font stacks resolve to the `--font-*` variables so any CSS
 *   utility (`font-mono`, `font-serif`) honours the typography choices.
 */
export default {
  darkMode: ["class"],
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  theme: {
    container: {
      center: true,
      padding: "1rem",
      screens: { "2xl": "1280px" },
    },
    extend: {
      fontFamily: {
        sans: ["var(--font-body)"],
        mono: ["var(--font-mono)"],
        serif: ["var(--font-serif)"],
      },
      colors: {
        // Shadcn-compatible aliases that primitives reference.
        border: "hsl(var(--border))",
        input: "hsl(var(--input))",
        ring: "hsl(var(--ring))",
        background: "hsl(var(--background))",
        foreground: "hsl(var(--foreground))",
        primary: {
          DEFAULT: "hsl(var(--primary))",
          foreground: "hsl(var(--primary-foreground))",
        },
        secondary: {
          DEFAULT: "hsl(var(--secondary))",
          foreground: "hsl(var(--secondary-foreground))",
        },
        muted: {
          DEFAULT: "hsl(var(--muted))",
          foreground: "hsl(var(--muted-foreground))",
        },
        accent: {
          DEFAULT: "hsl(var(--accent))",
          foreground: "hsl(var(--accent-foreground))",
        },
        destructive: {
          DEFAULT: "hsl(var(--destructive))",
          foreground: "hsl(var(--destructive-foreground))",
        },
        card: {
          DEFAULT: "hsl(var(--card))",
          foreground: "hsl(var(--card-foreground))",
        },
        popover: {
          DEFAULT: "hsl(var(--popover))",
          foreground: "hsl(var(--popover-foreground))",
        },

        // Brand families — match `colors_and_type.css` exactly.
        lake: {
          50: "#E8F1F4",
          100: "#C6DDE4",
          200: "#8FBCC9",
          300: "#4F94A6",
          500: "#1F6F86",
          700: "#144E62",
          900: "#0B3340",
        },
        ink: {
          300: "#B7C2CC",
          400: "#8295A6",
          500: "#56697C",
          700: "#2C3E50",
          800: "#1B2A38",
          900: "#0E1A24",
        },
        paper: {
          50: "#FBFAF6",
          100: "#F4F1EA",
          200: "#E8E3D6",
          300: "#D6CFBE",
        },
        roof: {
          100: "#F5DDC9",
          300: "#DD9F77",
          500: "#C26A3A",
          700: "#8E4720",
        },
        moss: {
          100: "#DDE6CF",
          500: "#6B8B4A",
          700: "#3F5B25",
        },
        sun: {
          100: "#FAEBB6",
          500: "#E5B33A",
          700: "#9B7416",
        },
      },
      borderRadius: {
        none: "0",
        sm: "2px",
        DEFAULT: "4px",
        md: "4px",
        lg: "8px",
        full: "9999px",
      },
      letterSpacing: {
        tight: "-0.02em",
        caps: "0.12em",
      },
      boxShadow: {
        "cl-1":
          "0 1px 0 rgba(14,26,36,0.04), 0 1px 2px rgba(14,26,36,0.06)",
        "cl-2":
          "0 2px 4px rgba(14,26,36,0.06), 0 4px 12px rgba(14,26,36,0.08)",
        "cl-3":
          "0 4px 8px rgba(14,26,36,0.08), 0 12px 32px rgba(14,26,36,0.12)",
      },
      transitionTimingFunction: {
        cl: "cubic-bezier(0.2, 0, 0, 1)",
      },
    },
  },
  plugins: [animate],
} satisfies Config;
