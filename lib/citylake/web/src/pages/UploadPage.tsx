import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Upload as UploadIcon } from "lucide-react";
import { useRef, useState, type FormEvent } from "react";
import { Link, useNavigate } from "react-router-dom";

import { Eyebrow } from "@/components/Eyebrow";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import { uploadDataset, type DatasetInfo } from "@/lib/api";

const NAME_PATTERN = /^[a-zA-Z0-9_]+$/;

export default function UploadPage() {
  const qc = useQueryClient();
  const navigate = useNavigate();

  const [file, setFile] = useState<File | null>(null);
  const [name, setName] = useState("");
  const [dragActive, setDragActive] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const mutation = useMutation<DatasetInfo, Error, void>({
    mutationFn: async () => {
      if (!file) throw new Error("Pick a file first");
      const trimmed = name.trim();
      if (!trimmed) throw new Error("Name the dataset first");
      if (!NAME_PATTERN.test(trimmed)) {
        throw new Error(
          "Dataset name can only contain letters, digits, and underscores (a-z, A-Z, 0-9, _).",
        );
      }
      return uploadDataset(trimmed, file);
    },
    onSuccess: (dataset) => {
      qc.invalidateQueries({ queryKey: ["datasets"] });
      navigate(`/datasets/${dataset.name}`);
    },
  });

  function pickFile(f: File | null) {
    setFile(f);
    if (!name && f?.name) {
      const cleaned = f.name
        .replace(/\.city\.jsonl?$/i, "")
        .replace(/\.fcb$/i, "")
        .replace(/[^a-zA-Z0-9_]/g, "_");
      setName(cleaned);
    }
  }

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    mutation.mutate();
  }

  return (
    <div className="space-y-8">
      <header className="space-y-1.5">
        <Eyebrow>Upload</Eyebrow>
        <h1 className="text-[40px] font-semibold leading-tight tracking-tight text-ink-900 font-sans">
          Upload a dataset
        </h1>
        <p className="text-[14px] text-ink-500 max-w-prose">
          Send a <code className="cl-code">.city.json</code>,{" "}
          <code className="cl-code">.city.jsonl</code>, or <code className="cl-code">.fcb</code>{" "}
          file to CityLake. The dataset holds every level of detail the source carries.
        </p>
      </header>

      <form onSubmit={onSubmit} className="space-y-5">
        <Card>
          <CardHeader>
            <Eyebrow>File</Eyebrow>
          </CardHeader>
          <CardContent>
            <div
              role="button"
              tabIndex={0}
              onClick={() => inputRef.current?.click()}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  inputRef.current?.click();
                }
              }}
              onDragOver={(e) => {
                e.preventDefault();
                setDragActive(true);
              }}
              onDragLeave={() => setDragActive(false)}
              onDrop={(e) => {
                e.preventDefault();
                setDragActive(false);
                const f = e.dataTransfer.files?.[0];
                if (f) pickFile(f);
              }}
              className={cn(
                "flex flex-col items-center justify-center gap-2 rounded-md border border-dashed border-paper-300 bg-paper-50 p-8 text-center transition-colors duration-150 ease-cl cursor-pointer",
                dragActive && "border-lake-500 bg-lake-50",
              )}
            >
              <UploadIcon className="h-7 w-7 text-ink-500" />
              {file ? (
                <p className="font-mono text-[13px] text-ink-900">{file.name}</p>
              ) : (
                <p className="text-[13px] text-ink-500">Drop a file or click to browse</p>
              )}
              <input
                ref={inputRef}
                type="file"
                className="hidden"
                accept=".json,.jsonl,.fcb,.city.json,.city.jsonl"
                onChange={(e) => pickFile(e.target.files?.[0] ?? null)}
              />
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <Eyebrow>Name</Eyebrow>
            <CardTitle className="text-[14px] mt-1 font-sans">
              This becomes the dataset&rsquo;s schema name
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            <Label htmlFor="name">Dataset name</Label>
            <Input
              id="name"
              mono
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="delft"
              pattern="[a-zA-Z0-9_]+"
              title="Letters, digits, and underscores only."
            />
            <p className="font-mono text-[11px] text-ink-500">
              Letters, digits, and underscores only (a-z, A-Z, 0-9, _).
            </p>
          </CardContent>
        </Card>

        <div className="flex items-center justify-between">
          <Button asChild variant="ghost">
            <Link to="/datasets">Cancel</Link>
          </Button>
          <Button type="submit" disabled={!file || !name.trim() || mutation.isPending}>
            {mutation.isPending ? "Uploading…" : "Upload"}
          </Button>
        </div>

        {mutation.error && (
          <Card accent="error">
            <CardContent className="pt-5 font-mono text-[12px] text-roof-700">
              {mutation.error.message}
            </CardContent>
          </Card>
        )}
      </form>
    </div>
  );
}
