//! A whole-object HTTP GET read as a STREAM, for the streaming formats'
//! identifier lookup (`cityjsonseq`, `citygml`).
//!
//! Locally both formats parse until the hit and stop; over HTTP a reader that
//! stops must also stop the TRANSFER, or the run reports a whole file it never
//! needed. [`BodyStream`] is a sync [`BufRead`] over one GET's response body:
//! it pulls the body chunk by chunk as the parser asks for bytes, counts the
//! bytes it RECEIVED, and, when dropped, drops the response — closing the
//! connection and abandoning the rest of the body.
//!
//! **What is counted, and why not through `CountingObjectStore`.** The
//! counting store tallies a request's range length when the request is made —
//! for a whole-object GET, the file size, whether or not the body is read. On
//! these cells that would report the whole file for a lookup that stopped at
//! 10 %, so they do not go through it: the request count is 1 by construction
//! (one `get`), and the byte count is the sum of the body chunks this reader
//! pulled off the connection ([`BodyStream::received`]). The two coincide on
//! a miss (the body is read to its end) and differ on a hit. Only the
//! received count reaches the CSV; nothing counts the request a second time.
//!
//! The count is chunk-granular: a chunk the parser pulled but did not finish
//! is counted whole, because it was received. Bytes the server wrote into
//! socket buffers that this client never pulled are not received and not
//! counted — the server's own total can exceed the client's by that much on
//! an aborted transfer (see `READ_BENCHMARK.md`'s network caveat).

use std::io::{BufRead, Read};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream::BoxStream;
use object_store::ObjectStoreExt;
use object_store::http::HttpBuilder;
use object_store::path::Path as ObjectPath;

/// One GET's response body as a sync [`BufRead`]. See the module docs.
pub(super) struct BodyStream {
    handle: tokio::runtime::Handle,
    stream: BoxStream<'static, object_store::Result<Bytes>>,
    chunk: Bytes,
    pos: usize,
    received: Arc<AtomicU64>,
}

impl BodyStream {
    /// Issues one whole-object GET of `key` under `base_url` and returns its
    /// body unread. Must be called outside an async context with a
    /// multi-thread runtime entered (`main` enters one for the HTTP
    /// transport): every [`BufRead::fill_buf`] blocks on the next chunk.
    pub(super) fn get(base_url: &str, key: &str) -> Result<Self> {
        let store = HttpBuilder::new()
            .with_url(base_url)
            .with_client_options(cityparquet_readbench::http_client::client_options())
            .build()?;
        let handle = tokio::runtime::Handle::current();
        let path = ObjectPath::from(key);
        let stream = handle
            .block_on(store.get(&path))
            .with_context(|| format!("GET {key}"))?
            .into_stream();
        Ok(Self {
            handle,
            stream,
            chunk: Bytes::new(),
            pos: 0,
            received: Arc::new(AtomicU64::new(0)),
        })
    }

    /// The bytes received so far, readable after the reader is dropped.
    pub(super) fn received(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.received)
    }
}

impl Read for BodyStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl BufRead for BodyStream {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        while self.pos == self.chunk.len() {
            match self.handle.block_on(self.stream.next()) {
                None => break,
                Some(Err(e)) => return Err(std::io::Error::other(e)),
                Some(Ok(chunk)) => {
                    self.received
                        .fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    self.chunk = chunk;
                    self.pos = 0;
                }
            }
        }
        Ok(&self.chunk[self.pos..])
    }

    fn consume(&mut self, amt: usize) {
        self.pos = (self.pos + amt).min(self.chunk.len());
    }
}

/// A [`BufRead`] that keeps a copy of every byte consumed from `inner`, so a
/// bounded look at a stream's start (a sniff, a header scan) can be replayed
/// ahead of the rest with [`Recording::replay`].
pub(super) struct Recording<R> {
    inner: R,
    log: Vec<u8>,
}

impl<R: BufRead> Recording<R> {
    pub(super) fn new(inner: R) -> Self {
        Self {
            inner,
            log: Vec::new(),
        }
    }

    /// What has been consumed so far.
    pub(super) fn log(&self) -> &[u8] {
        &self.log
    }

    /// The whole stream from its first byte: the recorded prefix, then the
    /// unread rest.
    pub(super) fn replay(self) -> std::io::Chain<std::io::Cursor<Vec<u8>>, R> {
        std::io::Cursor::new(self.log).chain(self.inner)
    }
}

impl<R: BufRead> Read for Recording<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let available = self.fill_buf()?;
        let n = available.len().min(buf.len());
        buf[..n].copy_from_slice(&available[..n]);
        self.consume(n);
        Ok(n)
    }
}

impl<R: BufRead> BufRead for Recording<R> {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.inner.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        if let Ok(available) = self.inner.fill_buf() {
            self.log
                .extend_from_slice(&available[..amt.min(available.len())]);
        }
        self.inner.consume(amt);
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use cityparquet_readbench::netsim::{NetProfile, NetSim};

    use super::super::citygml::CityGmlRunner;
    use super::super::cityjsonseq::CityJsonSeqRunner;
    use super::super::{FormatRunner, RunOutcome, Source as TransportSource};
    use crate::scenario::{QueryParams, Scenario};

    fn fixture(rel: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
    }

    /// One CityObject id per feature (the feature's own, else its first), in
    /// stream order.
    fn feature_ids(path: &Path) -> Vec<String> {
        let source = cityparquet::source::Source::open(path).unwrap();
        source
            .features()
            .unwrap()
            .map(|f| {
                let f = f.unwrap();
                if f.city_objects.contains_key(&f.id) {
                    f.id
                } else {
                    f.city_objects.keys().next().unwrap().clone()
                }
            })
            .collect()
    }

    fn lookup(runner: &dyn FormatRunner, source: &TransportSource, id: &str) -> RunOutcome {
        let params = QueryParams {
            target_id: Some(id.to_string()),
            ..QueryParams::default()
        };
        runner.run(source, Scenario::IdLookup, &params).unwrap()
    }

    /// `early_max_permille` bounds the early hit's transfer as a share of the
    /// file, from where the fixture's early object actually ends.
    /// `bandwidth_mbps` is chosen per fixture: the transfer can only stop
    /// short of the whole file if the file outlasts what is already in
    /// flight when the hit is parsed.
    ///
    /// Over net-sim, a streaming format's identifier lookup transfers only
    /// up to its hit, in one request, and returns what the local arm returns.
    fn streams_to_the_hit(
        runner: &dyn FormatRunner,
        path: &Path,
        bandwidth_mbps: f64,
        early_max_permille: u64,
    ) {
        let len = std::fs::metadata(path).unwrap().len();
        let ids = feature_ids(path);
        let dir = path.parent().unwrap();
        let key = path.file_name().unwrap().to_str().unwrap().to_string();
        let sim = NetSim::start(
            dir,
            NetProfile {
                bandwidth_mbps,
                latency_ms: 0.0,
            },
        )
        .unwrap();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let _entered = rt.enter();
        let http = TransportSource::Http {
            base_url: sim.base_url(),
            key: key.clone(),
        };
        let local = TransportSource::Local(path.to_path_buf());
        let early = &ids[ids.len() / 10];
        let late = &ids[ids.len() * 9 / 10];
        let mut received = Vec::new();
        for id in [early.as_str(), late.as_str(), "no-such-object"] {
            let over_http = lookup(runner, &http, id);
            let from_disk = lookup(runner, &local, id);
            assert_eq!(over_http.result_count, from_disk.result_count, "{key} {id}");
            let hit = id != "no-such-object";
            assert_eq!(over_http.result_count, u64::from(hit), "{key} {id}");
            assert_eq!(over_http.returned, from_disk.returned, "{key} {id}");
            let io = over_http.io.expect("HTTP reports its transfer");
            assert_eq!(io.requests, 1, "{key} {id}");
            received.push(io.bytes);
        }
        let [early_bytes, late_bytes, miss_bytes] = received[..] else {
            unreachable!()
        };
        assert!(
            early_bytes < len * early_max_permille / 1000,
            "{key}: early hit read {early_bytes} of {len}"
        );
        assert!(
            late_bytes >= early_bytes && late_bytes <= len,
            "{key}: late hit read {late_bytes} of {len}"
        );
        assert_eq!(miss_bytes, len, "{key}: a miss reads the whole file");
        // The server counts what it wrote; an aborted transfer can leave
        // bytes in flight that the client never received, never the reverse.
        let sent = sim.totals();
        assert_eq!(sent.requests, 3);
        assert!(sent.body_bytes >= received.iter().sum::<u64>());
    }

    #[test]
    fn cityjsonseq_id_lookup_stops_the_transfer_at_the_hit() {
        streams_to_the_hit(
            &CityJsonSeqRunner,
            &fixture("../../lib/cityparquet-rs/tests/fixtures/delft.city.jsonl"),
            10_000.0,
            500,
        );
    }

    #[test]
    fn citygml_id_lookup_stops_the_transfer_at_the_hit() {
        streams_to_the_hit(
            &CityGmlRunner,
            &fixture(
                "../../lib/cityparquet-rs/crates/core/tests/data/plateau_yokohama_bldg_fragment.gml",
            ),
            // Slow enough that the body arrives in net-sim's 16 KiB pieces
            // rather than as one read: a 90 KB file would otherwise be in
            // the client's buffer before the first member is parsed.
            8.0,
            // Its first member is ~82 % of the file: the transfer must stop
            // before the end, not before the half.
            1000,
        );
    }
}
