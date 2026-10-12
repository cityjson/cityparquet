//! The HTTP clients every format arm and the coordinator build, and what
//! the coordinator records about a real target.
//!
//! Every request names itself with [`USER_AGENT`], the corpus downloader's
//! own (`benchmark/scripts/corpus_bucket.py`): the hosted corpus sits behind
//! Cloudflare, which answers 403 to some default agents, so the benchmark
//! reads with the same identity that prepared its inputs.
//!
//! On a real target the coordinator also records the path it measured: the
//! base URL's host and the addresses it resolved to at run start, and per
//! object the `cf-cache-status` and `age` response headers, observed before
//! the first and after the last measured request ([`observe_cache`]).
//! object_store's `GetResult::attributes` carry only a fixed set of headers,
//! never these two, so they come from a separate one-byte ranged `GET`
//! (`Range: bytes=0-0`) with the same agent, outside the measured section.

use std::net::ToSocketAddrs;

use reqwest::header::{HeaderValue, RANGE};

/// The agent every benchmark request sends; the same string as
/// `USER_AGENT` in `benchmark/scripts/corpus_bucket.py`.
pub const USER_AGENT: &str = "cityparquet-bench-prep/1";

/// object_store's client options for every `HttpStore` the benchmark
/// builds: plain `http://` allowed (the simulated network), [`USER_AGENT`].
pub fn client_options() -> object_store::ClientOptions {
    object_store::ClientOptions::new()
        .with_allow_http(true)
        .with_user_agent(HeaderValue::from_static(USER_AGENT))
}

/// A `reqwest` client sending [`USER_AGENT`] (FlatCityBuf's range client,
/// the coordinator's size and cache probes).
pub fn reqwest_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .expect("a reqwest client with only a user agent set always builds")
}

/// `base_url`'s host and the addresses it resolves to now, sorted and
/// de-duplicated; an empty list when the lookup fails.
pub fn resolve_host(base_url: &str) -> (Option<String>, Vec<String>) {
    let Ok(url) = reqwest::Url::parse(base_url) else {
        return (None, Vec::new());
    };
    let host = url.host_str().map(str::to_string);
    let port = url.port_or_known_default().unwrap_or(443);
    let mut ips: Vec<String> = host
        .as_deref()
        .and_then(|h| (h, port).to_socket_addrs().ok())
        .map(|addrs| addrs.map(|a| a.ip().to_string()).collect())
        .unwrap_or_default();
    ips.sort();
    ips.dedup();
    (host, ips)
}

/// One object's cache headers as a one-byte ranged `GET` saw them:
/// `{"status", "cf_cache_status", "age"}` (a header absent is `null`), or
/// `{"error"}` when the request failed.
pub async fn observe_cache(client: &reqwest::Client, url: &str) -> serde_json::Value {
    let response = match client.get(url).header(RANGE, "bytes=0-0").send().await {
        Ok(r) => r,
        Err(e) => return serde_json::json!({ "error": e.to_string() }),
    };
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    serde_json::json!({
        "status": response.status().as_u16(),
        "cf_cache_status": header("cf-cache-status"),
        "age": header("age"),
    })
}

/// [`observe_cache`] for each of `urls`, on a runtime of its own.
pub fn observe_cache_all(urls: &[String]) -> Vec<serde_json::Value> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime builds");
    let client = reqwest_client();
    runtime.block_on(async {
        let mut out = Vec::with_capacity(urls.len());
        for url in urls {
            out.push(observe_cache(&client, url).await);
        }
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A one-shot-per-connection server answering every request with
    /// `cf-cache-status: HIT` and `age: 42`, recording each request head.
    async fn header_server() -> (String, Arc<Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let log = log.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let n = socket.read(&mut buf).await.unwrap_or(0);
                    log.lock()
                        .unwrap()
                        .push(String::from_utf8_lossy(&buf[..n]).to_lowercase());
                    let reply = "HTTP/1.1 206 Partial Content\r\ncontent-length: 1\r\n\
                                 content-range: bytes 0-0/10\r\ncf-cache-status: HIT\r\n\
                                 age: 42\r\nconnection: close\r\n\r\nx";
                    let _ = socket.write_all(reply.as_bytes()).await;
                });
            }
        });
        (format!("http://{addr}"), seen)
    }

    #[tokio::test]
    async fn observe_cache_reads_both_headers_and_sends_the_agent_and_a_one_byte_range() {
        let (base, seen) = header_server().await;
        let got = observe_cache(&reqwest_client(), &format!("{base}/a/b.gml")).await;
        assert_eq!(got["status"], 206);
        assert_eq!(got["cf_cache_status"], "HIT");
        assert_eq!(got["age"], "42");
        let head = seen.lock().unwrap()[0].clone();
        assert!(
            head.contains(&format!("user-agent: {USER_AGENT}")),
            "{head}"
        );
        assert!(head.contains("range: bytes=0-0"), "{head}");
    }

    #[tokio::test]
    async fn observe_cache_reports_absent_headers_as_null_and_failures_as_error() {
        let (base, _) = header_server().await;
        let ok = observe_cache(&reqwest_client(), &base).await;
        assert!(ok.get("error").is_none());
        let failed = observe_cache(&reqwest_client(), "http://127.0.0.1:1/x").await;
        assert!(failed["error"].is_string());
        assert!(failed.get("cf_cache_status").is_none());
    }

    #[tokio::test]
    async fn object_store_sends_the_agent() {
        use object_store::ObjectStoreExt;
        let (base, seen) = header_server().await;
        let store = object_store::http::HttpBuilder::new()
            .with_url(&base)
            .with_client_options(client_options())
            .build()
            .unwrap();
        let _ = store.get(&object_store::path::Path::from("x.gml")).await;
        let head = seen.lock().unwrap()[0].clone();
        assert!(
            head.contains(&format!("user-agent: {USER_AGENT}")),
            "{head}"
        );
    }

    #[test]
    fn resolve_host_names_the_host_and_its_addresses() {
        let (host, ips) = resolve_host("http://localhost:8080/bucket/v8");
        assert_eq!(host.as_deref(), Some("localhost"));
        assert!(
            ips.iter().any(|ip| ip == "127.0.0.1" || ip == "::1"),
            "{ips:?}"
        );
        assert_eq!(resolve_host("not a url"), (None, Vec::new()));
    }
}
