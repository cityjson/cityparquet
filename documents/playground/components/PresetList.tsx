import type { Preset } from "../presets";

interface PresetListProps {
  presets: readonly Preset[];
  activeId: string | null;
  onSelect: (preset: Preset) => void;
}

/**
 * The presets, grouped by the section each belongs to. The blurb is part of the
 * argument, not decoration: several of these queries are only impressive once
 * you know what they did *not* read.
 */
export default function PresetList({ presets, activeId, onSelect }: PresetListProps) {
  const groups = new Map<string, Preset[]>();
  for (const preset of presets) {
    const existing = groups.get(preset.group);
    if (existing) existing.push(preset);
    else groups.set(preset.group, [preset]);
  }

  return (
    <nav className="cp-presets" aria-label="Example queries">
      {[...groups].map(([group, items]) => (
        <section key={group} className="cp-preset-group">
          <h2 className="cp-preset-group-title">{group}</h2>
          <ul>
            {items.map((preset) => (
              <li key={preset.id}>
                <button
                  type="button"
                  className={preset.id === activeId ? "cp-preset cp-preset-active" : "cp-preset"}
                  onClick={() => onSelect(preset)}
                  aria-current={preset.id === activeId ? "true" : undefined}
                >
                  <span className="cp-preset-title">{preset.title}</span>
                  <span className="cp-preset-blurb">{preset.blurb}</span>
                </button>
              </li>
            ))}
          </ul>
        </section>
      ))}
    </nav>
  );
}
