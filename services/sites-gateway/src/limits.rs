//! The token bucket of each client key (workspace spec `site-gateway`, design D8).
//!
//! The key is `CF-Connecting-IP`, which Cloudflare overwrites at its edge; it selects a bucket and
//! is used for nothing else. An IPv4 address is its own key; an IPv6 address is keyed by its /64
//! prefix, because one client usually holds a whole /64. Every request without a valid address
//! shares one bucket. The table holds at most [`MAX_KEYS`] buckets; a new key in a full table evicts
//! the bucket that was used least recently, never the shared one, so many addresses cannot push a
//! client into the shared bucket.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;

use axum::http::HeaderMap;
use tokio::time::Instant;

/// The most client keys that the table holds.
const MAX_KEYS: usize = 4096;

/// The key of every request without a valid `CF-Connecting-IP`.
const SHARED: &str = "";

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    at: Instant,
}

/// The buckets of the client keys.
#[derive(Debug)]
pub(crate) struct Buckets {
    rate: f64,
    burst: f64,
    table: Mutex<HashMap<String, Bucket>>,
}

impl Buckets {
    /// Buckets that gain `rate_per_second` tokens a second and hold at most `burst`.
    pub(crate) fn new(rate_per_second: u32, burst: u32) -> Self {
        Self {
            rate: f64::from(rate_per_second.max(1)),
            burst: f64::from(burst.max(1)),
            table: Mutex::new(HashMap::new()),
        }
    }

    /// Take one token for the client of `headers`, or give back the whole seconds after which the
    /// client may try again.
    pub(crate) fn take(&self, headers: &HeaderMap) -> Result<(), u64> {
        let key = client_key(headers);
        let now = Instant::now();
        let Ok(mut buckets) = self.table.lock() else {
            return Ok(());
        };
        if !buckets.contains_key(&key) && buckets.len() >= MAX_KEYS {
            let oldest = buckets
                .iter()
                .filter(|(held, _)| held.as_str() != SHARED)
                .min_by_key(|(_, bucket)| bucket.at)
                .map(|(held, _)| held.clone());
            if let Some(oldest) = oldest {
                buckets.remove(&oldest);
            }
        }
        let bucket = buckets.entry(key).or_insert(Bucket {
            tokens: self.burst,
            at: now,
        });
        let tokens = refilled(*bucket, now, self.rate, self.burst);
        if tokens >= 1.0 {
            *bucket = Bucket {
                tokens: tokens - 1.0,
                at: now,
            };
            Ok(())
        } else {
            *bucket = Bucket { tokens, at: now };
            let wait = ((1.0 - tokens) / self.rate).ceil().max(1.0);
            Err(seconds(wait))
        }
    }
}

/// The tokens of `bucket` at `now`.
fn refilled(bucket: Bucket, now: Instant, rate: f64, burst: f64) -> f64 {
    let elapsed = now.saturating_duration_since(bucket.at).as_secs_f64();
    elapsed.mul_add(rate, bucket.tokens).min(burst)
}

/// `wait` seconds as a whole number of at least one.
fn seconds(wait: f64) -> u64 {
    std::time::Duration::try_from_secs_f64(wait.ceil()).map_or(1, |wait| wait.as_secs().max(1))
}

/// The client key of `headers`: the IPv4 address or the /64 prefix of the IPv6 address in
/// `CF-Connecting-IP`, or the shared key.
fn client_key(headers: &HeaderMap) -> String {
    let address = headers
        .get("cf-connecting-ip")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<IpAddr>().ok());
    match address {
        Some(IpAddr::V4(address)) => address.to_string(),
        Some(IpAddr::V6(address)) => address.to_ipv4_mapped().map_or_else(
            || {
                let [a, b, c, d, ..] = address.segments();
                format!("{a:x}:{b:x}:{c:x}:{d:x}::/64")
            },
            |mapped| mapped.to_string(),
        ),
        None => SHARED.to_owned(),
    }
}
