//! External merge sort of encoded rows by `(hilbert, row ordinal)`, for the
//! Hilbert row order with bounded memory.
//!
//! Rows arrive in source order, each with the Hilbert key of the feature it
//! came from. They are buffered until their Arrow memory exceeds the budget,
//! then sorted and spilled as one **run**: an Arrow IPC stream file holding
//! the rows plus the two key columns. At the end the runs are merged k-way.
//! The row ordinal makes every key unique, so the order is total and equals a
//! stable sort of whole features by key: a feature's rows share a key and
//! have consecutive ordinals, so its family stays contiguous (spec 02
//! "Feature families are contiguous").
//!
//! Both paths emit batches of exactly `batch_size` rows (the last one
//! shorter), so a spilled sort and an in-memory one write identical tables.
//! Runs are IPC *stream* files because `object_type` is dictionary-encoded and
//! its dictionary differs from batch to batch, which the stream format
//! carries and the file format does not.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::fs;
use std::io::{BufReader, BufWriter};
use std::path::PathBuf;
use std::sync::Arc;

use arrow_array::{Array, RecordBatch, UInt32Array, UInt64Array};
use arrow_ipc::CompressionType;
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::{IpcWriteOptions, StreamWriter};
use arrow_schema::{DataType, Field, Schema, SchemaRef};
use arrow_select::interleave::interleave_record_batch;
use cityparquet_schema::CityParquetError;

use crate::Result;

/// Rows per batch in a run file, at most: the merge holds one such batch per
/// run, so this, not the output batch size, bounds the merge's memory per run.
const RUN_BATCH: usize = 1024;
/// Bytes per batch in a run file, roughly at most, so a run of large rows
/// (LoD3 WKB runs to ~100 KB a row) does not make the merge's batch per run
/// large: the merge holds about `runs × RUN_BYTES`.
const RUN_BYTES: usize = 8 << 20;

const HILBERT: &str = "__cityparquet_hilbert";
const ORDINAL: &str = "__cityparquet_ordinal";

/// Buffers keyed batches and yields them sorted; see the module doc.
pub(crate) struct ExternalSorter {
    schema: SchemaRef,
    /// `schema` with the two key columns appended, as stored in a run.
    run_schema: SchemaRef,
    budget: usize,
    /// The directory the sorter's own run directory is made in.
    spill_root: PathBuf,
    /// The run directory, unique to this sorter: made on the first spill
    /// and removed, with the runs in it, on drop.
    run_dir: Option<tempfile::TempDir>,
    batch_size: usize,
    buffered: Vec<Keyed>,
    buffered_bytes: usize,
    next_ordinal: u64,
    runs: Vec<PathBuf>,
}

/// One buffered batch with its rows' keys; the ordinals are `first..`.
struct Keyed {
    batch: RecordBatch,
    hilbert: Vec<u32>,
    first: u64,
}

impl ExternalSorter {
    /// A sorter that spills once buffered rows exceed `budget` bytes, into a
    /// directory of its own made inside `spill_root` on the first spill and
    /// removed on drop. Nothing else in `spill_root` is touched.
    pub(crate) fn new(
        schema: SchemaRef,
        budget: usize,
        spill_root: PathBuf,
        batch_size: usize,
    ) -> Self {
        let mut fields: Vec<Arc<Field>> = schema.fields().iter().cloned().collect();
        fields.push(Arc::new(Field::new(HILBERT, DataType::UInt32, false)));
        fields.push(Arc::new(Field::new(ORDINAL, DataType::UInt64, false)));
        let run_schema = Arc::new(Schema::new_with_metadata(fields, schema.metadata().clone()));
        Self {
            schema,
            run_schema,
            budget,
            spill_root,
            run_dir: None,
            batch_size: batch_size.max(1),
            buffered: Vec::new(),
            buffered_bytes: 0,
            next_ordinal: 0,
            runs: Vec::new(),
        }
    }

    /// Adds `batch`, whose row `i` has Hilbert key `hilbert[i]`.
    pub(crate) fn push(&mut self, batch: RecordBatch, hilbert: Vec<u32>) -> Result<()> {
        debug_assert_eq!(batch.num_rows(), hilbert.len());
        let rows = batch.num_rows() as u64;
        self.buffered_bytes += batch.get_array_memory_size();
        self.buffered.push(Keyed {
            batch,
            hilbert,
            first: self.next_ordinal,
        });
        self.next_ordinal += rows;
        if self.buffered_bytes > self.budget {
            self.spill()?;
        }
        Ok(())
    }

    /// The buffered rows' `(batch, row)` positions in key order.
    fn sorted_positions(&self) -> Vec<(usize, usize)> {
        let mut keyed: Vec<(u32, u64, usize, usize)> =
            Vec::with_capacity(self.buffered.iter().map(|k| k.hilbert.len()).sum());
        for (b, k) in self.buffered.iter().enumerate() {
            for (row, &h) in k.hilbert.iter().enumerate() {
                keyed.push((h, k.first + row as u64, b, row));
            }
        }
        keyed.sort_unstable_by_key(|&(h, ordinal, _, _)| (h, ordinal));
        keyed.into_iter().map(|(_, _, b, row)| (b, row)).collect()
    }

    /// Sorts the buffered rows and writes them as one run.
    fn spill(&mut self) -> Result<()> {
        if self.buffered.is_empty() {
            return Ok(());
        }
        if self.run_dir.is_none() {
            let created = |e| {
                CityParquetError::io_source(
                    format!(
                        "cannot create a sort directory in {}",
                        self.spill_root.display()
                    ),
                    e,
                )
            };
            fs::create_dir_all(&self.spill_root).map_err(created)?;
            self.run_dir = Some(
                tempfile::Builder::new()
                    .prefix("cityparquet-sort-")
                    .tempdir_in(&self.spill_root)
                    .map_err(created)?,
            );
        }
        let run_dir = self.run_dir.as_ref().expect("made above").path();
        let path = run_dir.join(format!("run-{:05}.arrows", self.runs.len()));
        let file = fs::File::create(&path).map_err(|e| {
            CityParquetError::io_source(format!("cannot create {}", path.display()), e)
        })?;
        let options = IpcWriteOptions::default()
            .try_with_compression(Some(CompressionType::LZ4_FRAME))
            .map_err(ipc_err)?;
        let mut writer =
            StreamWriter::try_new_with_options(BufWriter::new(file), &self.run_schema, options)
                .map_err(ipc_err)?;
        let keyed: Vec<RecordBatch> = self
            .buffered
            .iter()
            .map(|k| {
                let ordinals: UInt64Array = (k.first..k.first + k.hilbert.len() as u64).collect();
                let mut columns = k.batch.columns().to_vec();
                columns.push(Arc::new(UInt32Array::from(k.hilbert.clone())));
                columns.push(Arc::new(ordinals));
                RecordBatch::try_new(Arc::clone(&self.run_schema), columns)
            })
            .collect::<std::result::Result<_, _>>()?;
        let refs: Vec<&RecordBatch> = keyed.iter().collect();
        // A row's size is estimated as its batch's average.
        let row_bytes: Vec<usize> = self
            .buffered
            .iter()
            .map(|k| k.batch.get_array_memory_size() / k.batch.num_rows().max(1))
            .collect();
        let positions = self.sorted_positions();
        let mut start = 0;
        while start < positions.len() {
            let mut end = start;
            let mut bytes = 0;
            while end < positions.len()
                && end - start < RUN_BATCH
                && (end == start || bytes < RUN_BYTES)
            {
                bytes += row_bytes[positions[end].0];
                end += 1;
            }
            writer
                .write(&interleave_record_batch(&refs, &positions[start..end])?)
                .map_err(ipc_err)?;
            start = end;
        }
        writer.finish().map_err(ipc_err)?;
        self.runs.push(path);
        self.buffered.clear();
        self.buffered_bytes = 0;
        Ok(())
    }

    /// All rows pushed, sorted, in batches of `batch_size` rows, and the
    /// number of runs spilled (`0` when every row fitted in the budget).
    pub(crate) fn finish(mut self) -> Result<(Sorted, usize)> {
        if self.runs.is_empty() {
            let positions = self.sorted_positions();
            let sorted = Sorted::Memory {
                batches: std::mem::take(&mut self.buffered)
                    .into_iter()
                    .map(|k| k.batch)
                    .collect(),
                positions,
                next: 0,
                batch_size: self.batch_size,
            };
            return Ok((sorted, 0));
        }
        self.spill()?;
        let mut cursors = Vec::with_capacity(self.runs.len());
        for path in &self.runs {
            let file = fs::File::open(path).map_err(|e| {
                CityParquetError::io_source(format!("cannot open {}", path.display()), e)
            })?;
            let reader = StreamReader::try_new(BufReader::new(file), None).map_err(ipc_err)?;
            cursors.push(RunCursor::new(reader, self.schema.fields().len())?);
        }
        let mut heap = BinaryHeap::new();
        for (run, cursor) in cursors.iter().enumerate() {
            if let Some(key) = cursor.key() {
                heap.push(Reverse((key, run)));
            }
        }
        let runs = self.runs.len();
        let sorted = Sorted::Merge(Merge {
            cursors,
            heap,
            batch_size: self.batch_size,
            schema: Arc::clone(&self.schema),
            _sorter: self,
        });
        Ok((sorted, runs))
    }
}

fn ipc_err(e: arrow_schema::ArrowError) -> CityParquetError {
    CityParquetError::io(format!("sort spill: {e}"))
}

/// The sorted rows, as an iterator of batches.
pub(crate) enum Sorted {
    Memory {
        batches: Vec<RecordBatch>,
        positions: Vec<(usize, usize)>,
        next: usize,
        batch_size: usize,
    },
    Merge(Merge),
}

impl Iterator for Sorted {
    type Item = Result<RecordBatch>;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Sorted::Memory {
                batches,
                positions,
                next,
                batch_size,
            } => {
                if *next >= positions.len() {
                    return None;
                }
                let end = (*next + *batch_size).min(positions.len());
                let refs: Vec<&RecordBatch> = batches.iter().collect();
                let out = interleave_record_batch(&refs, &positions[*next..end]);
                *next = end;
                Some(out.map_err(Into::into))
            }
            Sorted::Merge(merge) => merge.next_batch().transpose(),
        }
    }
}

/// A k-way merge over the spilled runs.
pub(crate) struct Merge {
    cursors: Vec<RunCursor>,
    heap: BinaryHeap<Reverse<((u32, u64), usize)>>,
    batch_size: usize,
    schema: SchemaRef,
    /// Kept so the run files outlive the merge.
    _sorter: ExternalSorter,
}

impl Merge {
    fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        // The run batches this output batch draws from, and the rows drawn.
        let mut sources: Vec<RecordBatch> = Vec::new();
        let mut source_of: Vec<Option<(u64, usize)>> = vec![None; self.cursors.len()];
        let mut positions: Vec<(usize, usize)> = Vec::with_capacity(self.batch_size);
        while positions.len() < self.batch_size {
            let Some(Reverse((_, run))) = self.heap.pop() else {
                break;
            };
            let cursor = &mut self.cursors[run];
            // Index of the cursor's current batch in `sources`.
            let source = match source_of[run] {
                Some((generation, index)) if generation == cursor.generation => index,
                _ => {
                    sources.push(cursor.batch.clone());
                    source_of[run] = Some((cursor.generation, sources.len() - 1));
                    sources.len() - 1
                }
            };
            positions.push((source, cursor.row));
            cursor.advance()?;
            if let Some(key) = cursor.key() {
                self.heap.push(Reverse((key, run)));
            }
        }
        if positions.is_empty() {
            return Ok(None);
        }
        let refs: Vec<&RecordBatch> = sources.iter().collect();
        let batch = interleave_record_batch(&refs, &positions)?;
        // The run batches already dropped their key columns.
        debug_assert_eq!(batch.schema().fields().len(), self.schema.fields().len());
        Ok(Some(batch))
    }
}

/// The read position in one run.
struct RunCursor {
    reader: StreamReader<BufReader<fs::File>>,
    /// Columns of the original schema; the key columns follow them.
    width: usize,
    batch: RecordBatch,
    hilbert: UInt32Array,
    ordinal: UInt64Array,
    row: usize,
    /// Bumped on every batch the cursor loads, so the merge knows when its
    /// reference to the cursor's batch is stale.
    generation: u64,
    done: bool,
}

impl RunCursor {
    fn new(reader: StreamReader<BufReader<fs::File>>, width: usize) -> Result<Self> {
        let mut cursor = Self {
            reader,
            width,
            batch: RecordBatch::new_empty(Arc::new(Schema::empty())),
            hilbert: UInt32Array::from(Vec::<u32>::new()),
            ordinal: UInt64Array::from(Vec::<u64>::new()),
            row: 0,
            generation: 0,
            done: false,
        };
        cursor.load()?;
        Ok(cursor)
    }

    fn load(&mut self) -> Result<()> {
        loop {
            match self.reader.next() {
                None => {
                    self.done = true;
                    return Ok(());
                }
                Some(batch) => {
                    let batch = batch.map_err(ipc_err)?;
                    if batch.num_rows() == 0 {
                        continue;
                    }
                    let column = |i: usize| Arc::clone(batch.column(i));
                    self.hilbert = column(self.width)
                        .as_any()
                        .downcast_ref::<UInt32Array>()
                        .cloned()
                        .ok_or_else(|| CityParquetError::io("sort spill: bad key column"))?;
                    self.ordinal = column(self.width + 1)
                        .as_any()
                        .downcast_ref::<UInt64Array>()
                        .cloned()
                        .ok_or_else(|| CityParquetError::io("sort spill: bad key column"))?;
                    self.batch = batch.project(&(0..self.width).collect::<Vec<_>>())?;
                    self.row = 0;
                    self.generation += 1;
                    return Ok(());
                }
            }
        }
    }

    fn key(&self) -> Option<(u32, u64)> {
        (!self.done).then(|| (self.hilbert.value(self.row), self.ordinal.value(self.row)))
    }

    fn advance(&mut self) -> Result<()> {
        self.row += 1;
        if self.row >= self.batch.num_rows() {
            self.load()?;
        }
        Ok(())
    }
}
