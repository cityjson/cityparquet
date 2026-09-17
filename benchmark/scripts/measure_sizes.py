#!/usr/bin/env python3
"""Record on-disk sizes for the five prepared format-comparison artefacts."""
from __future__ import annotations
import argparse, csv
from pathlib import Path
HEADER = ["dataset","format","bytes","mb","ratio_vs_cityjsonseq","baseline_format","ratio_vs_baseline"]
FORMATS = {"citygml":".gml", "cityjson":".city.json", "cityjsonseq":".city.jsonl", "flatcitybuf":".fcb", "cityparquet-hilbert":"-hilbert.parquet"}
def stem(path: Path) -> str:
 for suffix in (".city.jsonl", ".city.json", ".citygml", ".jsonl", ".json", ".gml", ".xml"):
  if path.name.endswith(suffix): return path.name[:-len(suffix)]
 return path.stem
def size(path: Path) -> int:
 return sum(item.stat().st_size for item in path.rglob("*") if item.is_file()) if path.is_dir() else path.stat().st_size
def main() -> None:
 p=argparse.ArgumentParser(); p.add_argument('--input',type=Path,required=True); p.add_argument('--prepared',type=Path,required=True); p.add_argument('--out',type=Path,required=True); a=p.parse_args()
 dataset=stem(a.input); rows=[]
 for fmt,suffix in FORMATS.items():
  artifact=a.prepared / f"{dataset}{suffix}"
  if not artifact.exists(): raise SystemExit(f"missing prepared {fmt} artefact: {artifact}")
  rows.append((fmt,size(artifact)))
 baseline=dict(rows)["cityjsonseq"]
 kept=[]
 if a.out.exists():
  with a.out.open(newline='') as s: kept=list(csv.reader(s))
  if kept and kept[0] != HEADER: raise SystemExit(f"unexpected size header in {a.out}")
  kept=[row for row in kept[1:] if row and row[0] != dataset]
 a.out.parent.mkdir(parents=True,exist_ok=True)
 with a.out.open('w',newline='') as s:
  w=csv.writer(s); w.writerow(HEADER); w.writerows(kept)
  for fmt,bytes_ in rows: w.writerow([dataset,fmt,bytes_,f"{bytes_/1048576:.6f}",f"{baseline/bytes_:.6f}","cityjsonseq",f"{baseline/bytes_:.6f}"])
if __name__ == '__main__': main()
