import { useMutation } from "@tanstack/react-query";
import { useState, type FormEvent } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { exportModule, type ExportFormat, type ModuleInfo } from "@/lib/api";

function errorMessage(err: unknown): string {
  return err instanceof Error ? err.message : String(err);
}

const FORMATS: { value: ExportFormat; label: string }[] = [
  { value: "cityjson", label: "CityJSON" },
  { value: "cityjsonseq", label: "CityJSONSeq" },
  { value: "flatcitybuf", label: "FlatCityBuf" },
];

/**
 * Exports one object module to a single CityJSON-family file. The output
 * path is resolved against the server's configured output root — the
 * dialog says so up front, rather than a user learning it from a 400.
 */
export function ExportDialog({ ds, modules }: { ds: string; modules: ModuleInfo[] }) {
  const objectModules = modules.filter((m) => m.role === "object");

  const [open, setOpen] = useState(false);
  const [module, setModule] = useState("");
  const [format, setFormat] = useState<ExportFormat>("cityjson");
  const [outputPath, setOutputPath] = useState("");

  const exportMutation = useMutation({
    mutationFn: () => exportModule(ds, { module, output_path: outputPath, format }),
  });

  function onOpenChange(next: boolean) {
    setOpen(next);
    if (!next) {
      setModule("");
      setFormat("cityjson");
      setOutputPath("");
      exportMutation.reset();
    }
  }

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    exportMutation.mutate();
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-4">
        <p className="text-[13px] text-ink-500">
          Write one module to a single CityJSON-family file.
        </p>
        <Button
          size="sm"
          variant="secondary"
          onClick={() => {
            exportMutation.reset();
            setOpen(true);
          }}
        >
          Export…
        </Button>
      </div>

      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Export a module</DialogTitle>
            <DialogDescription>
              Writes one module of <code className="cl-code">{ds}</code> to a single file. The
              output path is relative to the server&rsquo;s configured output root.
            </DialogDescription>
          </DialogHeader>

          {exportMutation.isSuccess ? (
            <>
              <p className="font-mono text-[12px] text-moss-700">Export completed.</p>
              <DialogFooter>
                <Button onClick={() => onOpenChange(false)}>Close</Button>
              </DialogFooter>
            </>
          ) : (
            <form onSubmit={onSubmit} className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="export-module">Module</Label>
                <select
                  id="export-module"
                  required
                  value={module}
                  onChange={(e) => setModule(e.target.value)}
                  className="flex h-9 w-full rounded-md border border-paper-300 bg-white px-3 py-2 text-[14px] text-ink-900 font-mono focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-lake-300 focus-visible:ring-offset-1 focus-visible:border-lake-500"
                >
                  <option value="">Choose a module…</option>
                  {objectModules.map((m) => (
                    <option key={m.name} value={m.name}>
                      {m.name}
                    </option>
                  ))}
                </select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="export-format">Format</Label>
                <select
                  id="export-format"
                  value={format}
                  onChange={(e) => setFormat(e.target.value as ExportFormat)}
                  className="flex h-9 w-full rounded-md border border-paper-300 bg-white px-3 py-2 text-[14px] text-ink-900 font-mono focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-lake-300 focus-visible:ring-offset-1 focus-visible:border-lake-500"
                >
                  {FORMATS.map((f) => (
                    <option key={f.value} value={f.value}>
                      {f.label}
                    </option>
                  ))}
                </select>
              </div>

              <div className="space-y-2">
                <Label htmlFor="export-output-path">Output path</Label>
                <Input
                  id="export-output-path"
                  mono
                  required
                  value={outputPath}
                  onChange={(e) => setOutputPath(e.target.value)}
                  placeholder="exports/delft-building.city.json"
                />
                <p className="font-mono text-[11px] text-ink-500">
                  Relative to the server&rsquo;s configured output root.
                </p>
              </div>

              {exportMutation.isError && (
                <p className="font-mono text-[12px] text-roof-700">
                  {errorMessage(exportMutation.error)}
                </p>
              )}

              <DialogFooter>
                <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
                  Cancel
                </Button>
                <Button type="submit" disabled={!module || !outputPath || exportMutation.isPending}>
                  {exportMutation.isPending ? "Exporting…" : "Export"}
                </Button>
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}
