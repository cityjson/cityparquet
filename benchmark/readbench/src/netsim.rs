//! A simulated-network HTTP/1.1 file server for the `network` benchmark family.
//!
//! It serves one directory over plain HTTP on `127.0.0.1` and imposes a
//! deterministic network profile on every response (no jitter, no loss):
//!
//! - **Latency.** After a request's headers arrive, the server waits
//!   `latency_ms` before it writes the first response byte. Concurrent
//!   requests wait concurrently, as they would on a real path.
//! - **Bandwidth.** Response bodies leave through ONE limiter at
//!   `bandwidth_mbps`, shared by every connection, so parallel requests divide
//!   the bandwidth rather than multiply it. The limiter is a virtual clock:
//!   each [`CHUNK`]-byte piece of a body reserves the next `len * 8 /
//!   bandwidth` seconds of the link and is written when its reservation ends.
//!   Reservations are chained, so timer lateness never accumulates. An idle
//!   link earns at most [`BURST_MS`] of credit (the burst): a body that starts
//!   after an idle period may begin that much earlier than a strictly serial
//!   link would allow. It absorbs the timer's resolution and is the only
//!   departure from `latency + bytes * 8 / bandwidth`.
//! - Headers, `HEAD` responses and error bodies are not charged to the
//!   bandwidth budget and are not counted in [`NetTotals::body_bytes`].
//!
//! HTTP spoken: `GET` and `HEAD`; a single byte range per request
//! (`bytes=a-b`, `bytes=a-`, `bytes=-n`) answered with `206` and
//! `Content-Range`, `416` when unsatisfiable; `Accept-Ranges`,
//! `Content-Length`, `ETag` and `Last-Modified` on every file response; `404`
//! for a missing file or a path that leaves the served directory; persistent
//! connections unless the client sends `Connection: close`. Multi-range
//! requests are refused with `501`: none of the benchmark's clients sends one
//! (object_store issues one ranged `GET` per range; `http-range-client` one
//! per buffered read). No TLS, no HTTP/2.
//!
//! [`NetTotals`] tallies requests, body bytes and accepted connections for the
//! run, so they can be checked against the clients' own `http_requests` and
//! `bytes_read`.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, UNIX_EPOCH};

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// The size of one paced piece of a response body.
pub const CHUNK: usize = 16 * 1024;
/// The idle credit the limiter grants, in milliseconds of link time.
pub const BURST_MS: f64 = 2.0;

/// A network profile: the two parameters the simulation imposes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetProfile {
    pub bandwidth_mbps: f64,
    pub latency_ms: f64,
}

impl NetProfile {
    /// The model time of a transfer: `bytes * 8 / bandwidth + requests *
    /// latency`, in seconds.
    pub fn model_secs(&self, bytes: u64, requests: u64) -> f64 {
        bytes as f64 * 8.0 / (self.bandwidth_mbps * 1e6) + requests as f64 * self.latency_ms / 1e3
    }
}

/// The server's own tallies for one run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NetTotals {
    pub requests: u64,
    pub body_bytes: u64,
    pub connections: u64,
}

#[derive(Default)]
struct Counters {
    requests: AtomicU64,
    body_bytes: AtomicU64,
    connections: AtomicU64,
}

/// The shared bandwidth budget: a virtual clock of when the link is next free.
struct Link {
    secs_per_byte: f64,
    next_free: Mutex<Option<Instant>>,
}

impl Link {
    /// Reserve `len` bytes of link time; returns when they may be written.
    fn reserve(&self, len: usize) -> Instant {
        let now = Instant::now();
        let credit = Duration::from_secs_f64(BURST_MS / 1e3);
        let mut next = self.next_free.lock().unwrap();
        let earliest = now.checked_sub(credit).unwrap_or(now);
        let start = match *next {
            Some(t) if t > earliest => t,
            _ => earliest,
        };
        let end = start + Duration::from_secs_f64(len as f64 * self.secs_per_byte);
        *next = Some(end);
        end
    }
}

struct Shared {
    root: PathBuf,
    profile: NetProfile,
    link: Link,
    counters: Counters,
}

/// A running simulated-network server. Dropping it stops the server.
pub struct NetSim {
    addr: std::net::SocketAddr,
    shared: Arc<Shared>,
    stop: Option<tokio::sync::oneshot::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl NetSim {
    /// Serve `root` under `profile` on an ephemeral `127.0.0.1` port, on a
    /// dedicated thread with its own runtime.
    pub fn start(root: &Path, profile: NetProfile) -> Result<NetSim> {
        Self::start_on(root, profile, 0)
    }

    /// As [`NetSim::start`], on a chosen port (`0` = ephemeral).
    pub fn start_on(root: &Path, profile: NetProfile, port: u16) -> Result<NetSim> {
        anyhow::ensure!(
            profile.bandwidth_mbps > 0.0 && profile.latency_ms >= 0.0,
            "a network profile needs bandwidth > 0 and latency >= 0, got {profile:?}"
        );
        let root = root
            .canonicalize()
            .with_context(|| format!("net-sim root {}", root.display()))?;
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", port))?;
        std_listener.set_nonblocking(true)?;
        let addr = std_listener.local_addr()?;
        let shared = Arc::new(Shared {
            root,
            profile,
            link: Link {
                secs_per_byte: 8.0 / (profile.bandwidth_mbps * 1e6),
                next_free: Mutex::new(None),
            },
            counters: Counters::default(),
        });
        let (tx, mut rx) = tokio::sync::oneshot::channel::<()>();
        let sh = shared.clone();
        let thread = std::thread::Builder::new()
            .name("net-sim".into())
            .spawn(move || {
                let rt = tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(4)
                    .enable_all()
                    .build()
                    .expect("net-sim runtime");
                rt.block_on(async move {
                    let listener = TcpListener::from_std(std_listener).expect("net-sim listener");
                    loop {
                        tokio::select! {
                            _ = &mut rx => break,
                            accepted = listener.accept() => {
                                if let Ok((stream, _)) = accepted {
                                    let _ = stream.set_nodelay(true);
                                    sh.counters.connections.fetch_add(1, Ordering::Relaxed);
                                    tokio::spawn(serve_conn(stream, sh.clone()));
                                }
                            }
                        }
                    }
                });
                rt.shutdown_timeout(Duration::from_millis(200));
            })?;
        Ok(NetSim {
            addr,
            shared,
            stop: Some(tx),
            thread: Some(thread),
        })
    }

    /// `http://127.0.0.1:<port>` — the `--base-url` for the readbench child.
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub fn profile(&self) -> NetProfile {
        self.shared.profile
    }

    /// The idle credit, in milliseconds (see the module docs).
    pub fn burst_ms(&self) -> f64 {
        BURST_MS
    }

    pub fn totals(&self) -> NetTotals {
        let c = &self.shared.counters;
        NetTotals {
            requests: c.requests.load(Ordering::Relaxed),
            body_bytes: c.body_bytes.load(Ordering::Relaxed),
            connections: c.connections.load(Ordering::Relaxed),
        }
    }
}

impl Drop for NetSim {
    fn drop(&mut self) {
        if let Some(tx) = self.stop.take() {
            let _ = tx.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Request {
    method: String,
    path: String,
    range: Option<String>,
    close: bool,
}

async fn read_request(r: &mut BufReader<tokio::net::tcp::OwnedReadHalf>) -> Option<Request> {
    let mut line = String::new();
    if r.read_line(&mut line).await.ok()? == 0 {
        return None;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    let http10 = parts.next() == Some("HTTP/1.0");
    let mut range = None;
    let mut close = http10;
    loop {
        let mut h = String::new();
        if r.read_line(&mut h).await.ok()? == 0 {
            return None;
        }
        let h = h.trim_end();
        if h.is_empty() {
            break;
        }
        if let Some((k, v)) = h.split_once(':') {
            let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
            match k.as_str() {
                "range" => range = Some(v.to_string()),
                "connection" => close = v.eq_ignore_ascii_case("close"),
                _ => {}
            }
        }
    }
    let path = target.split('?').next().unwrap_or("").to_string();
    Some(Request {
        method,
        path,
        range,
        close,
    })
}

fn percent_decode(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Map a request path onto the served directory; `None` if it would leave it.
fn resolve(root: &Path, url_path: &str) -> Option<PathBuf> {
    let rel = PathBuf::from(percent_decode(url_path.trim_start_matches('/'))?);
    if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(root.join(rel))
}

enum RangeSpec {
    Full,
    Part(u64, u64),
    Unsatisfiable,
    Unsupported,
}

/// Parse one `Range` header against a file of `len` bytes (RFC 9110 §14).
fn parse_range(header: Option<&str>, len: u64) -> RangeSpec {
    let Some(h) = header else {
        return RangeSpec::Full;
    };
    let Some(spec) = h.trim().strip_prefix("bytes=") else {
        return RangeSpec::Full;
    };
    if spec.contains(',') {
        return RangeSpec::Unsupported;
    }
    let Some((a, b)) = spec.trim().split_once('-') else {
        return RangeSpec::Full;
    };
    let (a, b) = (a.trim(), b.trim());
    if a.is_empty() {
        return match b.parse::<u64>() {
            Ok(0) | Err(_) => RangeSpec::Unsatisfiable,
            Ok(_) if len == 0 => RangeSpec::Unsatisfiable,
            Ok(n) => RangeSpec::Part(len.saturating_sub(n), len - 1),
        };
    }
    let Ok(start) = a.parse::<u64>() else {
        return RangeSpec::Full;
    };
    if start >= len {
        return RangeSpec::Unsatisfiable;
    }
    let end = if b.is_empty() {
        len - 1
    } else {
        match b.parse::<u64>() {
            Ok(e) if e >= start => e.min(len - 1),
            _ => return RangeSpec::Full,
        }
    };
    RangeSpec::Part(start, end)
}

/// An RFC 9110 IMF-fixdate from seconds since the epoch.
fn http_date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    const WD: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MO: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        WD[days.rem_euclid(7) as usize],
        d,
        MO[(m - 1) as usize],
        y,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

async fn serve_conn(stream: TcpStream, sh: Arc<Shared>) {
    let (rd, mut wr) = stream.into_split();
    let mut rd = BufReader::new(rd);
    while let Some(req) = read_request(&mut rd).await {
        sh.counters.requests.fetch_add(1, Ordering::Relaxed);
        if sh.profile.latency_ms > 0.0 {
            tokio::time::sleep(Duration::from_secs_f64(sh.profile.latency_ms / 1e3)).await;
        }
        let keep = !req.close;
        if respond(&req, &mut wr, &sh).await.is_err() || !keep {
            break;
        }
    }
}

async fn simple(
    wr: &mut tokio::net::tcp::OwnedWriteHalf,
    status: &str,
    extra: &str,
    close: bool,
) -> std::io::Result<()> {
    let conn = if close { "close" } else { "keep-alive" };
    let head =
        format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: {conn}\r\n{extra}\r\n");
    wr.write_all(head.as_bytes()).await
}

async fn respond(
    req: &Request,
    wr: &mut tokio::net::tcp::OwnedWriteHalf,
    sh: &Shared,
) -> std::io::Result<()> {
    if req.method != "GET" && req.method != "HEAD" {
        return simple(
            wr,
            "405 Method Not Allowed",
            "Allow: GET, HEAD\r\n",
            req.close,
        )
        .await;
    }
    let meta = match resolve(&sh.root, &req.path) {
        Some(p) => tokio::fs::metadata(&p)
            .await
            .ok()
            .filter(|m| m.is_file())
            .map(|m| (p, m)),
        None => None,
    };
    let Some((path, meta)) = meta else {
        return simple(wr, "404 Not Found", "", req.close).await;
    };
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs());
    let (status, start, end) = match parse_range(req.range.as_deref(), len) {
        RangeSpec::Full => ("200 OK", 0, len),
        RangeSpec::Part(a, b) => ("206 Partial Content", a, b + 1),
        RangeSpec::Unsatisfiable => {
            let cr = format!("Content-Range: bytes */{len}\r\n");
            return simple(wr, "416 Range Not Satisfiable", &cr, req.close).await;
        }
        RangeSpec::Unsupported => {
            return simple(wr, "501 Not Implemented", "", req.close).await;
        }
    };
    let body_len = end - start;
    let content_range = if status.starts_with("206") {
        format!("Content-Range: bytes {}-{}/{len}\r\n", start, end - 1)
    } else {
        String::new()
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {body_len}\r\nAccept-Ranges: bytes\r\n\
         ETag: \"{len:x}-{mtime:x}\"\r\nLast-Modified: {}\r\n\
         Content-Type: application/octet-stream\r\n{content_range}Connection: {}\r\n\r\n",
        http_date(mtime),
        if req.close { "close" } else { "keep-alive" }
    );
    wr.write_all(head.as_bytes()).await?;
    if req.method == "HEAD" || body_len == 0 {
        return Ok(());
    }
    let mut file = tokio::fs::File::open(&path).await?;
    file.seek(std::io::SeekFrom::Start(start)).await?;
    let mut left = body_len;
    let mut buf = vec![0u8; CHUNK];
    while left > 0 {
        let n = (left as usize).min(CHUNK);
        file.read_exact(&mut buf[..n]).await?;
        let due = sh.link.reserve(n);
        tokio::time::sleep_until(due.into()).await;
        wr.write_all(&buf[..n]).await?;
        sh.counters
            .body_bytes
            .fetch_add(n as u64, Ordering::Relaxed);
        left -= n as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_date_matches_known_instants() {
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(1_445_412_480), "Wed, 21 Oct 2015 07:28:00 GMT");
    }

    #[test]
    fn model_time_adds_transfer_and_latency() {
        let p = NetProfile {
            bandwidth_mbps: 100.0,
            latency_ms: 20.0,
        };
        assert!((p.model_secs(12_500_000, 3) - 1.06).abs() < 1e-9);
    }
}
