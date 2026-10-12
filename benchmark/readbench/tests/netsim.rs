//! The simulated-network HTTP server (`cityparquet_readbench::netsim`): range
//! semantics, the latency and bandwidth model, the shared bandwidth budget and
//! the server's own request/byte totals.
//!
//! Timing assertions use small payloads and generous tolerances (a lower bound
//! at the model time minus the documented burst, an upper bound well above the
//! model) so a busy CI machine cannot fail them.

use std::path::Path;
use std::time::{Duration, Instant};

use cityparquet_readbench::netsim::{NetProfile, NetSim};

fn serve(dir: &Path, bandwidth_mbps: f64, latency_ms: f64) -> NetSim {
    NetSim::start(
        dir,
        NetProfile {
            bandwidth_mbps,
            latency_ms,
        },
    )
    .expect("start net-sim")
}

fn write_file(dir: &Path, name: &str, len: usize) -> Vec<u8> {
    let body: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
    std::fs::write(dir.join(name), &body).unwrap();
    body
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[test]
fn ranges_heads_and_404s_follow_http_semantics() {
    let dir = tempfile::tempdir().unwrap();
    let body = write_file(dir.path(), "a.bin", 1000);
    let sim = serve(dir.path(), 1000.0, 0.0);
    let url = format!("{}/a.bin", sim.base_url());
    rt().block_on(async {
        let c = reqwest::Client::new();
        let full = c.get(&url).send().await.unwrap();
        assert_eq!(full.status(), 200);
        assert_eq!(full.headers()["accept-ranges"], "bytes");
        assert_eq!(full.headers()["content-length"], "1000");
        assert!(full.headers().contains_key("etag"));
        assert!(full.headers().contains_key("last-modified"));
        assert_eq!(full.bytes().await.unwrap().as_ref(), &body[..]);

        let r = c
            .get(&url)
            .header("Range", "bytes=10-19")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 206);
        assert_eq!(r.headers()["content-range"], "bytes 10-19/1000");
        assert_eq!(r.bytes().await.unwrap().as_ref(), &body[10..20]);

        let open = c
            .get(&url)
            .header("Range", "bytes=990-")
            .send()
            .await
            .unwrap();
        assert_eq!(open.headers()["content-range"], "bytes 990-999/1000");
        assert_eq!(open.bytes().await.unwrap().len(), 10);

        let suffix = c
            .get(&url)
            .header("Range", "bytes=-5")
            .send()
            .await
            .unwrap();
        assert_eq!(suffix.status(), 206);
        assert_eq!(suffix.bytes().await.unwrap().as_ref(), &body[995..]);

        // A range running past the end is clipped, as RFC 9110 requires.
        let clip = c
            .get(&url)
            .header("Range", "bytes=995-5000")
            .send()
            .await
            .unwrap();
        assert_eq!(clip.headers()["content-range"], "bytes 995-999/1000");

        let bad = c
            .get(&url)
            .header("Range", "bytes=2000-")
            .send()
            .await
            .unwrap();
        assert_eq!(bad.status(), 416);
        assert_eq!(bad.headers()["content-range"], "bytes */1000");

        let head = c.head(&url).send().await.unwrap();
        assert_eq!(head.status(), 200);
        assert_eq!(head.headers()["content-length"], "1000");

        let missing = c
            .get(format!("{}/nope.bin", sim.base_url()))
            .send()
            .await
            .unwrap();
        assert_eq!(missing.status(), 404);
        // A client library normalises `..` away, so the escape is sent raw.
        use std::io::{Read, Write};
        let mut raw =
            std::net::TcpStream::connect(sim.base_url().trim_start_matches("http://")).unwrap();
        raw.write_all(b"GET /../a.bin HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut reply = String::new();
        raw.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 404"), "{reply}");
    });
    let totals = sim.totals();
    assert_eq!(totals.requests, 9);
    // Body bytes actually sent: 1000 + 10 + 10 + 5 + 5 (HEAD/404/416 send none
    // that count — error bodies are excluded).
    assert_eq!(totals.body_bytes, 1030);
}

#[test]
fn one_body_takes_latency_plus_size_over_bandwidth_at_two_rates() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "b.bin", 250_000);
    // 250 kB at 20 Mbps = 100 ms; at 10 Mbps = 200 ms; latency 30 ms.
    for (mbps, expect_ms) in [(20.0, 130.0), (10.0, 230.0)] {
        let sim = serve(dir.path(), mbps, 30.0);
        let url = format!("{}/b.bin", sim.base_url());
        let elapsed = rt().block_on(async {
            let c = reqwest::Client::new();
            let t = Instant::now();
            let b = c.get(&url).send().await.unwrap().bytes().await.unwrap();
            assert_eq!(b.len(), 250_000);
            t.elapsed()
        });
        let ms = elapsed.as_secs_f64() * 1e3;
        let burst_ms = sim.burst_ms();
        assert!(
            ms >= expect_ms - burst_ms - 2.0 && ms <= expect_ms * 1.6 + 30.0,
            "{mbps} Mbps: {ms:.1} ms, model {expect_ms} ms"
        );
    }
}

#[test]
fn sequential_ranged_requests_pay_latency_each() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "c.bin", 100_000);
    let sim = serve(dir.path(), 8.0, 25.0);
    let url = format!("{}/c.bin", sim.base_url());
    // 8 requests x 5 kB at 8 Mbps (= 1 byte per microsecond): 8 x 25 ms
    // latency + 40 kB x 1 us = 240 ms.
    let elapsed = rt().block_on(async {
        let c = reqwest::Client::new();
        let t = Instant::now();
        for k in 0..8u64 {
            let lo = k * 10_000;
            let r = format!("bytes={}-{}", lo, lo + 4_999);
            let b = c.get(&url).header("Range", r).send().await.unwrap();
            assert_eq!(b.bytes().await.unwrap().len(), 5_000);
        }
        t.elapsed()
    });
    let ms = elapsed.as_secs_f64() * 1e3;
    assert!(
        (200.0..=420.0).contains(&ms),
        "8 ranged requests: {ms:.1} ms, model 240 ms"
    );
    assert_eq!(sim.totals().requests, 8);
    assert_eq!(sim.totals().body_bytes, 40_000);
}

#[test]
fn concurrent_requests_share_one_bandwidth_budget() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "d.bin", 100_000);
    // Four concurrent 100 kB bodies at 16 Mbps (2 bytes/us): 400 kB share one
    // budget = 200 ms. Separate budgets would finish in 50 ms.
    let sim = serve(dir.path(), 16.0, 0.0);
    let url = format!("{}/d.bin", sim.base_url());
    let elapsed = rt().block_on(async {
        let c = reqwest::Client::new();
        let t = Instant::now();
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..4 {
            let (c, url) = (c.clone(), url.clone());
            set.spawn(async move {
                c.get(&url)
                    .send()
                    .await
                    .unwrap()
                    .bytes()
                    .await
                    .unwrap()
                    .len()
            });
        }
        while let Some(n) = set.join_next().await {
            assert_eq!(n.unwrap(), 100_000);
        }
        t.elapsed()
    });
    assert!(
        elapsed >= Duration::from_millis(170) && elapsed <= Duration::from_millis(400),
        "shared budget: {elapsed:?}, model 200 ms"
    );
    assert_eq!(sim.totals().body_bytes, 400_000);
}

#[test]
fn keep_alive_reuses_one_connection() {
    let dir = tempfile::tempdir().unwrap();
    write_file(dir.path(), "e.bin", 100);
    let sim = serve(dir.path(), 1000.0, 0.0);
    let url = format!("{}/e.bin", sim.base_url());
    rt().block_on(async {
        let c = reqwest::Client::new();
        for _ in 0..5 {
            c.get(&url).send().await.unwrap().bytes().await.unwrap();
        }
    });
    let t = sim.totals();
    assert_eq!(t.requests, 5);
    assert_eq!(t.connections, 1);
}
