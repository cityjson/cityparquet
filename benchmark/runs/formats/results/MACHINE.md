# Measurement host

Captured by benchmark/scripts/machine_record.sh at 2026-10-27T15:09:02Z.

```
Linux 6.8.0-142-generic x86_64
Architecture:                            x86_64
CPU op-mode(s):                          32-bit, 64-bit
Address sizes:                           48 bits physical, 48 bits virtual
Byte Order:                              Little Endian
CPU(s):                                  128
On-line CPU(s) list:                     0-127
Vendor ID:                               AuthenticAMD
Model name:                              AMD EPYC 7542 32-Core Processor
CPU family:                              23
Model:                                   49
Thread(s) per core:                      2
Core(s) per socket:                      32
Socket(s):                               2
Stepping:                                0
Frequency boost:                         enabled
               total        used        free      shared  buff/cache   available
Mem:     540725084160 19692003328 257002700800   959737856 270570115072 521033080832
rustc 1.93.1 (01f6ddf75 2026-02-11)
cargo 1.93.1 (083ac5135 2025-12-15)
git e695262f0e1cc550fffcdcdea23ca44ce074b64d
```

## Isolation

What benchmark/README.md, "Running on a shared host", applies, read at the
start of the run. Each result CSV's `.params.json` records what was applied to
that run under `isolation`.

| Fact              | Value                                                                                                      |
| ----------------- | ---------------------------------------------------------------------------------------------------------- |
| numactl           | found                                                                                                      |
| taskset           | found                                                                                                      |
| Allowed CPUs      | 0-127 (Cpus_allowed_list of /proc/self/status)                                                             |
| NUMA node         | 0 (cores 0-31,64-95; those in the allowed CPUs are usable: coordinator on the first, children on the rest) |
| Pinning command   | numactl --all --physcpubind=<usable node 0 cores minus the first> --membind=0                              |
| Memory limit      | requested MemoryMax=64000000000 (decimal bytes, 64 GB) via systemd-run --user --scope                      |
| cgroup delegation | user@1020.service delegates: memory                                                                        |
| CPU governor      | schedutil                                                                                                  |
| SMT               | 1 (/sys/devices/system/cpu/smt/active: 1 = on)                                                             |
| Kernel            | Linux 6.8.0-142-generic                                                                                    |
| Other users' CPU  | 881.8% (summed ps pcpu)                                                                                    |
| MemTotal          | 540725084160 bytes                                                                                         |
| MemAvailable      | 521040080896 bytes (at start)                                                                              |

## Tool versions

| Tool               | Version                                                                              |
| ------------------ | ------------------------------------------------------------------------------------ |
| cityparquet-rs     | 68c158cbc96bbf1f2f485970e71902a54405f261 (last commit touching lib/cityparquet-rs)   |
| cjseq              | cjseq 0.3.1                                                                          |
| fcb CLI            | fcb 0.7.8                                                                            |
| fcb_core           | 0.7.6 (readbench Cargo.lock pin)                                                     |
| citygml-tools      | not found                                                                            |
| DuckDB             | v1.3.2 (Ossivalis) 0b83e5d2f6                                                        |
| PostgreSQL/PostGIS | image postgis/postgis; host psql: psql (PostgreSQL) 18.6 (Ubuntu 18.6-1.pgdg24.04+2) |
| 3DCityDB           | image 3dcitydb/3dcitydb-pg:16-3.4-5.1.2-alpine                                       |
| cjdb               | not found                                                                            |
