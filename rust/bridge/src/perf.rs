//! Response headers + compression + slow-request log (port of asgi_perf.py),
//! and ETag revalidation for the JSON lists the apps re-fetch on every launch.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::middleware::Next;
use axum::response::Response;
use std::time::{Duration, Instant};
use tower_http::compression::predicate::Predicate;

const SLOW: Duration = Duration::from_secs(2);

pub async fn headers_and_timing(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    for (k, v) in [
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "no-referrer"),
        ("strict-transport-security", "max-age=31536000; includeSubDomains"),
    ] {
        h.entry(k).or_insert(HeaderValue::from_static(v));
    }
    if !path.starts_with("/admin") {
        h.entry("content-security-policy")
            .or_insert(HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"));
    }
    let elapsed = start.elapsed();
    if elapsed > SLOW {
        tracing::warn!("slow request: {method} {path} took {:.2}s (status {})", elapsed.as_secs_f64(), res.status().as_u16());
    }
    res
}

/// Gzip only text-like bodies of 1 KiB or more, never ranges or 204/206/304.
#[derive(Clone, Copy, Default)]
pub struct Compressible;

impl Predicate for Compressible {
    fn should_compress<B>(&self, response: &axum::http::Response<B>) -> bool
    where
        B: axum::body::HttpBody,
    {
        let st = response.status();
        if matches!(st, StatusCode::NO_CONTENT | StatusCode::PARTIAL_CONTENT | StatusCode::NOT_MODIFIED) {
            return false;
        }
        let h: &HeaderMap = response.headers();
        if h.contains_key("content-encoding") || h.contains_key("content-range") {
            return false;
        }
        if let Some(n) = response.body().size_hint().exact() {
            if n < 1024 {
                return false;
            }
        }
        let ct = h.get("content-type").and_then(|v| v.to_str().ok()).unwrap_or("").to_ascii_lowercase();
        ["application/json", "text/", "application/javascript", "application/xml", "image/svg+xml"]
            .iter()
            .any(|p| ct.starts_with(p))
    }
}

/// GET routes whose JSON the apps poll (launch, pull-to-refresh, sync): they
/// answer with an ETag, and a matching If-None-Match gets an empty 304.
const ETAG_PATHS: &[&str] = &[
    "/user/playlists",
    "/user/favorites",
    "/user/settings",
    "/user/appearance",
    "/user/sync",
    "/user/music",
    "/user/music/metadata",
    "/user/download-history",
    "/user/queue",
    "/user/storage/usage",
    "/api/library/server",
    "/user/announcements",
];
/// Bodies larger than this are passed through untouched (no buffering).
const ETAG_MAX_BODY: usize = 8 * 1024 * 1024;

fn etag_route(path: &str) -> bool {
    ETAG_PATHS.contains(&path) || path.starts_with("/user/playlists/")
}

/// Weak ETag over the response body. Runs inside compression, so the hash
/// is of the JSON itself and stays the same whatever encoding is negotiated.
pub async fn etag(req: Request<Body>, next: Next) -> Response {
    use axum::http::{Method, header};
    if req.method() != Method::GET || !etag_route(req.uri().path()) {
        return next.run(req).await;
    }
    let inm = req.headers().get(header::IF_NONE_MATCH).and_then(|v| v.to_str().ok()).map(str::to_string);
    let res = next.run(req).await;
    let json = res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("application/json"));
    if res.status() != StatusCode::OK || !json || res.headers().contains_key(header::ETAG) {
        return res;
    }
    let (mut parts, body) = res.into_parts();
    let bytes = match axum::body::to_bytes(body, ETAG_MAX_BODY).await {
        Ok(b) => b,
        Err(_) => {
            parts.status = StatusCode::INTERNAL_SERVER_ERROR;
            return Response::from_parts(parts, Body::empty());
        }
    };
    use sha2::Digest;
    let digest = sha2::Sha256::digest(&bytes);
    let tag = format!("W/\"{}\"", hex::encode(&digest[..12]));
    // Per-account data: caches may keep it but must revalidate every time.
    parts.headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    parts.headers.insert(header::VARY, HeaderValue::from_static("Authorization, X-Account-Token, Accept-Encoding"));
    if let Ok(v) = HeaderValue::from_str(&tag) {
        parts.headers.insert(header::ETAG, v);
    }
    let matches = inm.is_some_and(|h| h.split(',').map(str::trim).any(|t| t == tag || t == "*" || t.trim_start_matches("W/") == tag.trim_start_matches("W/")));
    if matches {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::empty());
    }
    parts.headers.insert(header::CONTENT_LENGTH, HeaderValue::from(bytes.len()));
    Response::from_parts(parts, Body::from(bytes))
}

#[cfg(test)]
mod etag_tests {
    use super::etag_route;

    #[test]
    fn routes_covered() {
        assert!(etag_route("/user/music"));
        assert!(etag_route("/user/playlists"));
        assert!(etag_route("/user/playlists/abc"));
        assert!(!etag_route("/user/music/stream"));
        assert!(!etag_route("/user/musicx"));
        assert!(!etag_route("/api/stream/proxy"));
    }
}
