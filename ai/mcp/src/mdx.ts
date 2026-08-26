// Reducing the documentation site's MDX to the Markdown the corpus carries.

export interface Section {
  readonly heading: string;
  readonly level: number;
  readonly body: string;
}

export interface ReducedDoc {
  readonly title: string;
  readonly description: string;
  readonly body: string;
  readonly sections: readonly Section[];
}

const FRONTMATTER = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/;

/** Only the two scalar keys the corpus needs; the sidebar block is ignored. */
function frontmatterField(block: string, key: string): string {
  const match = new RegExp(`^${key}:\\s*(.+)$`, "m").exec(block);
  if (!match) return "";
  return match[1]!.trim().replace(/^["']|["']$/g, "");
}

/**
 * Blume admonitions carry normative text often enough that dropping them would
 * lose specification content, so they are unwrapped rather than stripped.
 */
function unwrapAdmonitions(markdown: string): string {
  return markdown.replace(
    /^:::[a-z]+(?:\[([^\]]*)\])?\r?\n([\s\S]*?)^:::[ \t]*$/gm,
    (_all, title: string | undefined, body: string) =>
      title ? `**${title}**\n\n${body.trim()}\n` : `${body.trim()}\n`,
  );
}

interface FenceTracker {
  isInside(line: string): boolean;
  fork(): FenceTracker;
  getState(): string | null;
  setState(state: string | null): void;
}

/** Tracks whether we are inside a fenced code block. Supports forking for speculative scans. */
function createFenceTracker(initialState: string | null = null): FenceTracker {
  let fence: string | null = initialState;
  return {
    isInside(line: string): boolean {
      const fenceMatch = /^\s*(```+|~~~+)/.exec(line);
      if (fenceMatch) {
        const marker = fenceMatch[1]!;
        if (fence === null) {
          fence = marker;
          return true;
        } else if (marker.startsWith(fence[0]!) && marker.length >= fence.length) {
          fence = null;
          return true;
        }
        return true;
      }
      return fence !== null;
    },
    fork(): FenceTracker {
      return createFenceTracker(fence);
    },
    getState(): string | null {
      return fence;
    },
    setState(state: string | null): void {
      fence = state;
    },
  };
}

/**
 * Unwraps an inline JSX element that is genuinely closed on the same line —
 * a self-closing `<Foo bar="1" />`, or a paired `<Badge …>text</Badge>` whose
 * matching close appears before the line ends — keeping its text content and
 * dropping the tags. Never touches a backtick-delimited code span, which is
 * where the specification's own type notation (`` `LIST<VARCHAR>` ``,
 * `` `STRUCT<… LIST<LIST<INT>>>` ``) lives.
 *
 * Two independent guards keep that notation intact even though it shares the
 * `<`/`>` characters: neither regex below matches without an actual closing
 * tag or a literal `/>`, which `LIST<VARCHAR>` never has; and both are only
 * ever applied outside backtick spans in the first place, so a construct
 * that *did* look closed while quoted (there is none in the specification
 * today) would still survive.
 */
function unwrapInlineJsx(line: string): string {
  const parts = line.split(/(`[^`]*`)/);
  for (let index = 0; index < parts.length; index += 2) {
    parts[index] = parts[index]!
      .replace(/<([A-Z][\w.]*)(?:\s[^<>]*?)?\/>/g, "")
      .replace(/<([A-Z][\w.]*)(?:\s[^<>]*?)?>([^<>]*)<\/\1>/g, "$2");
  }
  return parts.join("");
}

function stripJsx(markdown: string): string {
  const lines = markdown.split(/\r?\n/);
  const result: string[] = [];
  const tracker = createFenceTracker();
  let i = 0;

  while (i < lines.length) {
    let line = lines[i]!;

    // Track fence state
    const wasInFence = tracker.isInside(line);

    // Inside a fence: preserve everything as-is
    if (wasInFence) {
      result.push(line);
      i++;
      continue;
    }

    // Strip single-line import/export statements (only outside fences)
    if (/^import\s+[^\n]*$/.test(line)) {
      i++;
      continue;
    }
    if (/^export\s+[^\n]*$/.test(line)) {
      i++;
      continue;
    }

    // Strip single-line JSX elements (only outside fences)
    if (/^[ \t]*<\/?[A-Z][\w.]*(?:\s[^>]*)?>[ \t]*$/.test(line)) {
      i++;
      continue;
    }

    // Unwrap inline JSX elements mixed into a line of prose (only outside fences).
    line = unwrapInlineJsx(line);

    // Handle multi-line JSX: opening tag that closes on a later line (only outside fences)
    const multilineJsxStart = /^[ \t]*<[A-Z][\w.]*(?:\s|$)/.test(line);
    if (multilineJsxStart && !line.includes(">")) {
      // Consume lines until we find the closing >
      let j = i + 1;
      while (j < lines.length) {
        const nextLine = lines[j]!;
        if (nextLine.includes(">")) {
          // Found the closing >; skip all these lines and continue
          i = j + 1;
          break;
        }
        j++;
      }
      if (j === lines.length) {
        // Never found a closing >, treat as content
        result.push(line);
        i++;
      }
      continue;
    }

    // Handle comments: strip line-local comments only; preserve multi-line if they cross fences
    // Check if this line starts a multi-line comment without closing on same line
    if (line.includes("{/*") && !line.includes("*/}")) {
      // Scan for closing using a forked tracker to detect boundary crossings
      let j = i + 1;
      const lookahead = tracker.fork();
      const commentStartInFence = lookahead.getState() !== null;

      while (j < lines.length) {
        const nextLine = lines[j]!;
        lookahead.isInside(nextLine); // Advance the forked tracker

        if (nextLine.includes("*/}")) {
          // Found closing marker
          const commentEndInFence = lookahead.getState() !== null;

          // Preserve the entire comment if it spans fence boundaries
          if (commentStartInFence !== commentEndInFence) {
            result.push(line);
            for (let k = i + 1; k < j; k++) {
              result.push(lines[k]!);
            }
            result.push(nextLine);
            // Sync the forked state back to the main tracker
            tracker.setState(lookahead.getState());
          }
          // Otherwise it's entirely outside fences and was never added
          i = j + 1;
          break;
        }
        j++;
      }
      if (j === lines.length) {
        // No closing found: preserve the line as content
        result.push(line);
        i++;
      }
      continue;
    }

    // Strip line-local comments (those that open and close on the same line)
    let content = line.replace(/\{\/\*.*?\*\/\}/g, "");

    // Add non-empty content or empty lines to preserve structure
    if (content.trim() || line.trim() === "") {
      result.push(content);
    }

    i++;
  }

  return result.join("\n");
}

function absolutiseLinks(markdown: string, siteBaseUrl: string): string {
  return markdown.replace(/\]\((\/[^)\s]*)\)/g, (_all, path: string) => `](${siteBaseUrl}${path})`);
}

/** Heading scan that respects fenced code blocks, where `##` is not a heading. */
export function splitSections(markdown: string): Section[] {
  const lines = markdown.split(/\r?\n/);
  const sections: Section[] = [];
  let current: { heading: string; level: number; body: string[] } | null = null;
  const tracker = createFenceTracker();

  for (const line of lines) {
    const inFence = tracker.isInside(line);

    const heading = !inFence ? /^(#{2,6})\s+(.*)$/.exec(line) : null;
    if (heading) {
      if (current) sections.push({ ...current, body: current.body.join("\n") });
      current = { heading: heading[2]!.trim(), level: heading[1]!.length, body: [] };
      continue;
    }
    current?.body.push(line);
  }
  if (current) sections.push({ ...current, body: current.body.join("\n") });
  return sections;
}

export function reduceMdx(source: string, options: { siteBaseUrl: string }): ReducedDoc {
  const frontmatter = FRONTMATTER.exec(source);
  const block = frontmatter?.[1] ?? "";
  const rest = frontmatter ? source.slice(frontmatter[0].length) : source;

  const body = absolutiseLinks(stripJsx(unwrapAdmonitions(rest)), options.siteBaseUrl)
    .replace(/\n{3,}/g, "\n\n")
    .trim();

  return {
    title: frontmatterField(block, "title"),
    description: frontmatterField(block, "description"),
    body,
    sections: splitSections(body),
  };
}
