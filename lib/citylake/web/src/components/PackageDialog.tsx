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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { errorMessage, writePackage, type PackageFile } from "@/lib/api";

/**
 * Writes the dataset out as a CityParquet package directory, relative to
 * the server's configured output root. The written-files table on success
 * is the point of the feature: it is how a user learns the package is what
 * they expected, so it stays in the dialog rather than being reduced to a
 * one-line "done".
 */
export function PackageDialog({ ds }: { ds: string }) {
  const [open, setOpen] = useState(false);
  const [outputDir, setOutputDir] = useState("");

  const writeMutation = useMutation({
    mutationFn: () => writePackage(ds, outputDir),
  });

  function onOpenChange(next: boolean) {
    setOpen(next);
    if (!next) {
      setOutputDir("");
      writeMutation.reset();
    }
  }

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    writeMutation.mutate();
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-4">
        <p className="text-[13px] text-ink-500">
          Write <code className="cl-code">{ds}</code> out as a CityParquet package directory.
        </p>
        <Button size="sm" variant="secondary" onClick={() => setOpen(true)}>
          Write package…
        </Button>
      </div>

      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Write package</DialogTitle>
            <DialogDescription>
              Writes <code className="cl-code">{ds}</code> out as a CityParquet package directory.
              The output directory is relative to the server&rsquo;s configured output root.
            </DialogDescription>
          </DialogHeader>

          {writeMutation.isSuccess ? (
            <>
              <PackageFilesTable files={writeMutation.data} />
              <DialogFooter>
                <Button onClick={() => onOpenChange(false)}>Close</Button>
              </DialogFooter>
            </>
          ) : (
            <form onSubmit={onSubmit} className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="package-output-dir">Output directory</Label>
                <Input
                  id="package-output-dir"
                  mono
                  required
                  value={outputDir}
                  onChange={(e) => setOutputDir(e.target.value)}
                  placeholder="packages/delft"
                />
                <p className="font-mono text-[11px] text-ink-500">
                  Relative to the server&rsquo;s configured output root.
                </p>
              </div>

              {writeMutation.isError && (
                <p className="font-mono text-[12px] text-roof-700">
                  {errorMessage(writeMutation.error)}
                </p>
              )}

              <DialogFooter>
                <Button type="button" variant="secondary" onClick={() => onOpenChange(false)}>
                  Cancel
                </Button>
                <Button type="submit" disabled={!outputDir || writeMutation.isPending}>
                  {writeMutation.isPending ? "Writing…" : "Write package"}
                </Button>
              </DialogFooter>
            </form>
          )}
        </DialogContent>
      </Dialog>
    </div>
  );
}

function PackageFilesTable({ files }: { files: PackageFile[] }) {
  if (files.length === 0) {
    return <p className="font-mono text-[12px] text-moss-700">No files written.</p>;
  }

  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>File</TableHead>
          <TableHead>Action</TableHead>
          <TableHead className="text-right">Rows</TableHead>
          <TableHead className="text-right">Bytes</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {files.map((f) => (
          <TableRow key={f.file}>
            <TableCell className="font-mono text-[12px]">{f.file}</TableCell>
            <TableCell className="text-ink-500">{f.action}</TableCell>
            <TableCell className="text-right font-mono text-[12px] text-ink-500">
              {f.rows.toLocaleString()}
            </TableCell>
            <TableCell className="text-right font-mono text-[12px] text-ink-500">
              {f.bytes.toLocaleString()}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}
