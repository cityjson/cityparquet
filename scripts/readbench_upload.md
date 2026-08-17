# Uploading readbench artefacts for HTTP transport

`cityparquet-readbench run --transport http` (and its own `--child
--transport http`) needs a real public HTTPS endpoint supporting `Range`
requests — S3, Cloudflare R2, or any static host that serves byte-range
GETs. This repo does not automate the upload (no bundled server, no CI
integration — see `bench/READ_BENCHMARK.md`'s "HTTP transport" section for
why): do it once, by hand, per dataset you want to benchmark over HTTP.

## What to upload

Exactly the directory `just readbench-prepare <input>` (or
`readbench_prepare.sh`) produces — `bench/data/readbench/` by default —
preserving its structure:

```
bench/data/readbench/
  <name>.parquet/           # CityParquet package directory
    metadata.json           # STAC manifest — CityParquet's HTTP reader
    building.parquet        #   range-fetches this first to find the
    ...                     #   package's own table(s)
  <name>-hilbert.parquet/
    metadata.json
    ...
  <name>.fcb
  <name>.city.jsonl
  <name>.jsonl.gz
```

Nothing outside that directory is ever fetched: every format — the plain
(non-gz) `cityjsonseq` included — reads an artefact the prepare script wrote
there, under the name `Format::artefact` resolves
(`crates/cityparquet-readbench/src/format.rs`). The `--input` argument only
names the dataset.

Upload the **whole directory as-is** — don't rename or flatten anything;
`cityparquet-readbench`'s HTTP paths key off these exact relative names
(`<name>.parquet/metadata.json`, `<name>.fcb`, `<name>.city.jsonl`, …).

## Cloudflare R2 (`rclone`)

One-time setup (an R2 API token with object read/write scope):

```sh
rclone config create r2 s3 \
    provider=Cloudflare \
    access_key_id=<YOUR_ACCESS_KEY_ID> \
    secret_access_key=<YOUR_SECRET_ACCESS_KEY> \
    endpoint=https://<ACCOUNT_ID>.r2.cloudflarestorage.com
```

Upload, then make the bucket's objects public (either a public bucket, or a
custom domain / `r2.dev` public URL — see R2's own dashboard: Settings →
Public access):

```sh
rclone copy bench/data/readbench/ r2:<BUCKET_NAME>/readbench/ --progress
```

Your `--base-url` is the bucket's public root plus the `readbench/` prefix.
**Copy the exact Public Bucket URL from the R2 dashboard** (Settings →
Public access → the `r2.dev` URL shown there, e.g.
`https://pub-<id>.r2.dev`, or your own custom domain if you attached one) —
don't guess its shape from the bucket/account name, the managed `r2.dev`
domain is an opaque identifier R2 assigns, not `<BUCKET_NAME>.<ACCOUNT_ID>.
r2.dev`. Append `/readbench` to whatever that dashboard URL is; no trailing
slash needed, `cityparquet-readbench` joins `base_url/key` itself.

## AWS S3 (`aws s3`)

```sh
aws s3 sync bench/data/readbench/ s3://<BUCKET_NAME>/readbench/
```

Make the objects publicly readable — either a bucket policy allowing
`s3:GetObject` for `arn:aws:s3:::<BUCKET_NAME>/readbench/*`, or upload with
`--acl public-read` (only if the bucket doesn't block public ACLs):

```sh
aws s3 sync bench/data/readbench/ s3://<BUCKET_NAME>/readbench/ --acl public-read
```

`--base-url` is then the bucket's public HTTPS endpoint plus the prefix,
e.g. `https://<BUCKET_NAME>.s3.<REGION>.amazonaws.com/readbench`.

## Verifying `Range` support before running the benchmark

Confirm the host actually honours byte-range requests (S3/R2 do by
default; a plain static-file host usually does too, but check once per new
host):

```sh
curl -sS -D - -o /dev/null --range 0-99 \
    "<BASE_URL>/<name>.parquet/metadata.json"
```

(A plain `curl -I` sends a `HEAD` request, which some servers answer
without honouring `Range` at all even when a real ranged `GET` — what this
benchmark actually issues — works fine; `--range` on a normal `GET` is the
only way to check what the benchmark itself will experience.)

Expect `HTTP/1.1 206 Partial Content` with a `Content-Range` header (not
`200 OK`, which means the server ignored the `Range` header and would send
the whole file every time, defeating the point of the comparison).

## Running the benchmark

Once uploaded and verified, the prepared artefacts must still also be
present *locally* (the coordinator's own `QueryParams` derivation — dataset
bbox, sampled attribute/id — always reads the local `--prepared-dir`
directly, regardless of transport; see `bench/READ_BENCHMARK.md`):

```sh
cargo run --release -p cityparquet-readbench -- run \
    --input tests/fixtures/delft.city.jsonl \
    --prepared-dir bench/data/readbench \
    --out bench/read_results/delft-http.csv \
    --transport http --base-url "<BASE_URL>" \
    --repeat 7
```
