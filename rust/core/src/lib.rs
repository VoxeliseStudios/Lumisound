//! Lumisound app core, shared by the iOS, tvOS and watchOS apps and the
//! widget. Exposed to Swift through UniFFI (module `LumisoundCore`): every
//! functional system lives here; Swift keeps SwiftUI views, the app shell and
//! the handful of Apple APIs only Swift can reach.

uniffi::setup_scaffolding!();

pub mod aidj;
pub mod aria_lyrics;
pub mod blend;
pub mod analysis;
pub mod artresolve;
pub mod blobcache;
pub mod bridge;
pub mod streamcache;
pub mod appearance;
pub mod applock;
pub mod cache;
pub mod client;
pub mod crossdevice;
pub mod downloads;
pub mod dropdown;
pub mod gallery;
pub mod hub;
pub mod haptics;
pub mod haptic_brain;
pub mod intro;
pub mod duplicates;
pub mod dupes;
pub mod engine;
pub mod import;
pub mod library;
pub mod live;
pub mod live_activity;
pub mod lock;
pub mod lumiclip;
pub mod logger;
pub mod lyrics;
pub mod presets;
pub mod metadata;
pub mod mood;
pub mod perf;
pub mod quality;
pub mod radio;
pub mod error;
pub mod retry;
pub mod shuffle;
pub mod strata;
pub mod search;
pub mod smart;
pub mod store;
pub mod stack;
pub mod sync;
pub mod syncmerge;
pub mod thermal;
pub mod urlenc;
pub mod together;
pub mod stores;

use std::future::Future;
use std::sync::LazyLock;

/// One runtime for networking and background work. Calls from Swift spawn
/// their future here and await the JoinHandle, so they work whichever
/// executor polls them.
pub(crate) static RT: LazyLock<tokio::runtime::Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).thread_name("lumisound-core").enable_all().build().expect("tokio runtime")
});

pub(crate) async fn on_rt<F, T>(f: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: Send + 'static,
{
    RT.spawn(f).await.expect("core task panicked")
}

/// Core library version (Settings / diagnostics).
#[uniffi::export]
pub fn core_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Lowercase hex SHA-256 of `text` (UTF-8): stable keys for folder covers and
/// Rich Presence artwork, shared with the bridge and the RPC daemon.
#[uniffi::export]
pub fn sha256_hex(text: String) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(text.as_bytes()))
}

#[cfg(test)]
mod lib_tests {
    #[test]
    fn sha256_hex_matches_known_vector() {
        assert_eq!(super::sha256_hex("abc".into()), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    }
}
