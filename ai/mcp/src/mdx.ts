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

/** Tracks whether we are inside a fenced code block. */
function createFenceTracker(): { isInside(line: string): boolean } {
  let fence: string | null = null;
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
  };
}

function stripJsx(markdown: string): string {
  // First, strip MDX comments globally (they can span multiple lines)
  let text = markdown.replace(/\{\/\*[\s\S]*?\*\/\}/g, "");

  const tracker = createFenceTracker();
  const lines = text.split(/\r?\n/);
  const result: string[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i]!;

    // Track fence state
    if (tracker.isInside(line)) {
      result.push(line);
      i++;
      continue;
    }

    // Strip single-line import/export statements
    if (/^import\s+[^\n]*$/.test(line)) {
      i++;
      continue;
    }
    if (/^export\s+[^\n]*$/.test(line)) {
      i++;
      continue;
    }

    // Strip single-line JSX elements (complete on one line)
    if (/^[ \t]*<\/?[A-Z][\w.]*(?:\s[^>]*)?>[ \t]*$/.test(line)) {
      i++;
      continue;
    }

    // Handle multi-line JSX: opening tag that closes on a later line
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

    result.push(line);
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
  let fence: string | null = null;

  for (const line of lines) {
    const fenceMatch = /^\s*(```+|~~~+)/.exec(line);
    if (fenceMatch) {
      const marker = fenceMatch[1]!;
      if (fence === null) fence = marker;
      else if (marker.startsWith(fence[0]!) && marker.length >= fence.length) fence = null;
    }

    const heading = fence === null ? /^(#{2,6})\s+(.*)$/.exec(line) : null;
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
