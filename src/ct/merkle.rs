//! Merkle verification against a static-CT log's signed tree.
//!
//! The checkpoint signature says the log signed *some* tree. It says nothing
//! about whether the entries just read are in that tree, or whether the tree
//! is an extension of the one seen a minute ago. Those are separate claims,
//! and this module checks them from the log's own hash tiles:
//!
//! * **inclusion** — every ingested entry's leaf hash is in the signed tree.
//! * **consistency** — the new tree is an append-only extension of the last
//!   one, so the log has not rewritten history.
//!
//! Both cost extra fetches and CPU, which is why they are opt-in.

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use bytes::Bytes;
use parking_lot::Mutex;
use tlog_tiles::{
    check_tree, prove_tree, record_hash, stored_hash_index, Hash, Tile, TileHashReader, TileReader,
};
use tracing::debug;

/// Static-CT tiles are 256 wide, so height 8.
const TILE_HEIGHT: u8 = 8;

/// Full hash tiles kept per watcher. Each is 8 KiB, so this caps the cache at
/// a few MiB. Verification walks the right edge of the tree, so the working
/// set is the tiles along one root path plus the ones being proven.
const TILE_CACHE_CAPACITY: usize = 256;

/// Insertion-ordered map with a hard capacity: the oldest tile is evicted
/// when a new one arrives. Verification reads tiles in tree order and rarely
/// revisits an old one, so insertion order is a good enough eviction order
/// and costs no per-hit bookkeeping.
#[derive(Default)]
struct TileCache {
    entries: HashMap<String, Vec<u8>>,
    order: VecDeque<String>,
}

impl TileCache {
    fn get(&self, path: &str) -> Option<&Vec<u8>> {
        self.entries.get(path)
    }

    fn insert(&mut self, path: String, data: Vec<u8>) {
        if self.entries.contains_key(&path) {
            return;
        }
        while self.order.len() >= TILE_CACHE_CAPACITY {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
        self.order.push_back(path.clone());
        self.entries.insert(path, data);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Reads a log's hash tiles over HTTP, on demand, for the tlog verifier.
///
/// `TileReader` is synchronous, so this is meant to be driven from a blocking
/// task; `handle.block_on` is legal there and would panic on a runtime worker.
pub struct HttpTileReader {
    base_url: String,
    client: reqwest::Client,
    timeout: Duration,
    handle: tokio::runtime::Handle,
    /// The watcher's per-operator token bucket. Verification issues extra
    /// requests to the same host as ingest, so they have to come out of the
    /// same budget — without this the operator sees a second, unpaced client
    /// and answers it with 429s, which read as unverifiable tiles.
    rate_limiter: Option<crate::ct::OperatorRateLimiter>,
    /// Tiles the verifier has confirmed against the tree hash. Unverified
    /// downloads are never cached — that is the contract `save_tiles` exists
    /// to enforce, and caching a forged tile would poison every later proof.
    ///
    /// Bounded, and holds only full tiles. A full tile is immutable, so a hit
    /// is always valid; a partial tile changes width as the log grows, so
    /// caching one is both useless and a way for the map to accumulate an
    /// entry per width the log passed through.
    verified: Mutex<TileCache>,
}

impl HttpTileReader {
    pub fn new(
        base_url: &str,
        client: reqwest::Client,
        timeout: Duration,
        handle: tokio::runtime::Handle,
        rate_limiter: Option<crate::ct::OperatorRateLimiter>,
    ) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
            timeout,
            handle,
            rate_limiter,
            verified: Mutex::new(TileCache::default()),
        }
    }

    fn fetch(&self, path: &str) -> Result<Vec<u8>, tlog_tiles::Error> {
        let url = format!("{}/{}", self.base_url, path);
        let client = self.client.clone();
        let timeout = self.timeout;
        let fetch_url = url.clone();
        let limiter = self.rate_limiter.clone();
        self.handle
            .block_on(async move {
                if let Some(limiter) = &limiter {
                    limiter.tick().await;
                }
                let resp = client.get(&fetch_url).timeout(timeout).send().await.ok()?;
                if !resp.status().is_success() {
                    debug!(url = %fetch_url, status = %resp.status(), "hash tile request rejected");
                    return None;
                }
                resp.bytes().await.ok()
            })
            .map(|b| b.to_vec())
            .ok_or_else(|| {
                debug!(%url, "hash tile fetch failed");
                tlog_tiles::Error::InvalidTile
            })
    }
}

/// Where a hash tile lives on a static-CT log.
///
/// Not `Tile::path()`: that emits the Go sumdb layout `tile/<H>/<L>/<N>`,
/// which carries the tile height as a path segment. static-ct-api omits it —
/// `tile/<L>/<N>[.p/<W>]` — so using the crate's own path would 404 against
/// every log.
fn tile_path(tile: &Tile) -> String {
    let n = crate::ct::static_ct::encode_tile_path(tile.level_index());
    let width = tile.width();
    if width == 1u32 << tile.height() {
        format!("tile/{}/{}", tile.level(), n)
    } else {
        format!("tile/{}/{}.p/{}", tile.level(), n, width)
    }
}

impl TileReader for HttpTileReader {
    fn height(&self) -> u8 {
        TILE_HEIGHT
    }

    fn read_tiles(&self, tiles: &[Tile]) -> Result<Vec<Vec<u8>>, tlog_tiles::Error> {
        let mut out = Vec::with_capacity(tiles.len());
        for tile in tiles {
            let path = tile_path(tile);
            if let Some(cached) = self.verified.lock().get(&path) {
                out.push(cached.clone());
                continue;
            }
            let data = self.fetch(&path)?;
            // The verifier rejects a wrong-length tile itself, but catching it
            // here names the tile in the log line.
            let want = tile.width() as usize * 32;
            if data.len() != want {
                debug!(path = %path, got = data.len(), want, "hash tile has the wrong length");
                return Err(tlog_tiles::Error::InvalidTile);
            }
            out.push(data);
        }
        Ok(out)
    }

    fn save_tiles(&self, tiles: &[Tile], data: &[Vec<u8>]) {
        let mut cache = self.verified.lock();
        for (tile, bytes) in tiles.iter().zip(data) {
            if tile.width() != 1u32 << tile.height() {
                continue;
            }
            cache.insert(tile_path(tile), bytes.clone());
        }
    }
}

/// RFC 6962 `MerkleTreeLeaf` = version(0) || leaf_type(0) || TimestampedEntry.
///
/// Built from the tile's own bytes rather than from the parsed fields: the
/// hash has to be over what the log signed, not over a faithful-looking
/// re-encoding of it.
pub fn leaf_hash(timestamped_entry: &Bytes) -> Hash {
    let mut leaf = Vec::with_capacity(2 + timestamped_entry.len());
    leaf.extend_from_slice(&[0, 0]);
    leaf.extend_from_slice(timestamped_entry);
    record_hash(&leaf)
}

/// The result of a Merkle check.
///
/// `Unavailable` and `Mismatch` are deliberately different answers, and the
/// caller must treat them differently. The log not serving a tile — a 429, a
/// partial tile the log has already moved past — says nothing about the data;
/// a hash that disagrees says the data is not what was signed. The project
/// takes the same position on checkpoint signatures: inability to verify is
/// not proof of forgery.
#[derive(Debug)]
pub enum Verdict {
    Verified,
    Unavailable(String),
    Mismatch(String),
}

/// Check that every `(index, timestamped_entry)` is the entry the signed tree
/// holds at that index.
///
/// `TileHashReader` authenticates the tiles it reads against `root`, so a
/// hash that comes back is already proven to be in that tree; the remaining
/// step is confirming it is the hash of the entry we actually ingested.
pub fn verify_inclusion(
    reader: &HttpTileReader,
    tree_size: u64,
    root: Hash,
    entries: &[(u64, Bytes)],
) -> Verdict {
    if entries.is_empty() {
        return Verdict::Verified;
    }

    let hashes = TileHashReader::new(tree_size, root, reader);
    let indexes: Vec<u64> = entries
        .iter()
        .map(|(index, _)| stored_hash_index(0, *index))
        .collect();

    let in_tree = match tlog_tiles::HashReader::read_hashes(&hashes, &indexes) {
        Ok(h) => h,
        // The reader could not assemble an authenticated view of the tree.
        // Near the head that is routinely the log having moved past the
        // partial tile widths this checkpoint implies.
        Err(e) => {
            return Verdict::Unavailable(format!(
                "could not read leaf hashes from the signed tree: {e}"
            ))
        }
    };

    for ((index, entry), signed) in entries.iter().zip(in_tree) {
        if leaf_hash(entry) != signed {
            return Verdict::Mismatch(format!(
                "entry {index} is not the entry the signed tree holds at that index"
            ));
        }
    }
    Verdict::Verified
}

/// Check that the tree of size `new_size` extends the tree of size
/// `old_size` — that the log appended rather than rewrote.
pub fn verify_consistency(
    reader: &HttpTileReader,
    old_size: u64,
    old_root: Hash,
    new_size: u64,
    new_root: Hash,
) -> Verdict {
    if old_size == 0 {
        return Verdict::Verified;
    }
    // A tree is consistent with itself only if it is in fact the same tree.
    // Equal sizes with different roots is a log that rewrote an entry in
    // place, which is exactly what this check exists to catch — returning
    // early on the size alone would wave it through.
    if old_size == new_size {
        return if old_root == new_root {
            Verdict::Verified
        } else {
            Verdict::Mismatch(format!(
                "tree size {new_size} is unchanged but its root is not; the log rewrote history"
            ))
        };
    }
    // Not a fetch problem: a log that shrank has rewritten its tree.
    if new_size < old_size {
        return Verdict::Mismatch(format!(
            "tree shrank from {old_size} to {new_size}; a log may only append"
        ));
    }

    let hashes = TileHashReader::new(new_size, new_root, reader);
    let proof = match prove_tree(new_size, old_size, &hashes) {
        Ok(p) => p,
        Err(e) => return Verdict::Unavailable(format!("could not build a consistency proof: {e}")),
    };
    match check_tree(&proof, new_size, new_root, old_size, old_root) {
        Ok(()) => Verdict::Verified,
        Err(e) => Verdict::Mismatch(format!(
            "tree {new_size} does not extend tree {old_size}: {e}"
        )),
    }
}

pub fn parse_root_hash(base64: &str) -> Result<Hash, String> {
    Hash::parse_hash(base64).map_err(|e| format!("malformed checkpoint root hash: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tlog_tiles::{node_hash, HashReader};

    /// A tiny in-memory tree so the verifier can be exercised without a
    /// network or a live log.
    struct MemoryHashes(Vec<Hash>);

    impl HashReader for MemoryHashes {
        fn read_hashes(&self, indexes: &[u64]) -> Result<Vec<Hash>, tlog_tiles::Error> {
            indexes
                .iter()
                .map(|i| {
                    self.0
                        .get(*i as usize)
                        .copied()
                        .ok_or(tlog_tiles::Error::IndexesNotInTree)
                })
                .collect()
        }
    }

    fn build(records: &[&[u8]]) -> MemoryHashes {
        let mut stored: Vec<Hash> = Vec::new();
        // `stored_hashes` takes the record index, which is not the stored-hash
        // count: an append writes one leaf hash plus any interior hashes it
        // completes.
        for (index, data) in records.iter().enumerate() {
            let hashes =
                tlog_tiles::stored_hashes(index as u64, data, &MemoryHashes(stored.clone()))
                    .unwrap();
            stored.extend(hashes);
        }
        MemoryHashes(stored)
    }

    /// The bytes hashed must be `0x00 0x00 || TimestampedEntry`, and the
    /// prefix must not be dropped or doubled.
    /// A hash that disagrees is an integrity failure; a tile the log will not
    /// serve is not. Collapsing the two would either drop good data on every
    /// rate-limited fetch or wave through a forged entry.
    #[tokio::test]
    async fn an_unreachable_log_and_a_rewritten_tree_are_different_verdicts() {
        let records: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d"];
        let full = build(&records);
        let root = tlog_tiles::tree_hash(4, &full).unwrap();
        let older = tlog_tiles::tree_hash(2, &full).unwrap();

        let reader = HttpTileReader::new(
            "https://tiles.invalid",
            reqwest::Client::new(),
            Duration::from_millis(50),
            tokio::runtime::Handle::current(),
            None,
        );

        // A tree that shrank is a rewrite, decided without a single fetch.
        assert!(matches!(
            verify_consistency(&reader, 9, root, 4, root),
            Verdict::Mismatch(_)
        ));

        // A log we cannot reach is unavailable, not a forgery. Run it off the
        // runtime worker, since the reader blocks on its fetches.
        let verdict = tokio::task::spawn_blocking(move || {
            verify_consistency(&reader, 2, older, 4, root)
        })
        .await
        .unwrap();
        assert!(
            matches!(verdict, Verdict::Unavailable(_)),
            "unreachable tiles must not read as an integrity failure: {verdict:?}"
        );
    }

    /// The layout bug this cost a live run to find: `Tile::path()` is the Go
    /// sumdb form and includes the tile height, which static-ct logs do not
    /// serve.
    #[test]
    fn hash_tile_paths_use_the_static_ct_layout() {
        let full = Tile::new(TILE_HEIGHT, 0, 1234, 256, false);
        assert_eq!(tile_path(&full), "tile/0/x001/234");
        assert!(
            full.path().starts_with("tile/8/"),
            "the crate's own path carries the height; ours must not"
        );

        let level_one = Tile::new(TILE_HEIGHT, 1, 5, 256, false);
        assert_eq!(tile_path(&level_one), "tile/1/005");

        let partial = Tile::new(TILE_HEIGHT, 0, 7, 42, false);
        assert_eq!(tile_path(&partial), "tile/0/007.p/42");
    }

    #[test]
    fn a_leaf_hash_is_over_the_merkle_tree_leaf_encoding() {
        let entry = Bytes::from_static(b"timestamped-entry-bytes");
        let mut expected_input = vec![0u8, 0u8];
        expected_input.extend_from_slice(&entry);

        assert_eq!(leaf_hash(&entry), record_hash(&expected_input));
        assert_ne!(leaf_hash(&entry), record_hash(&entry));
    }

    #[test]
    fn a_root_hash_round_trips_through_base64() {
        let hash = record_hash(b"anything");
        let parsed = parse_root_hash(&hash.to_string()).unwrap();
        assert_eq!(parsed, hash);
        assert!(parse_root_hash("not base64!!").is_err());
        assert!(parse_root_hash("c2hvcnQ=").is_err(), "wrong length");
    }

    /// The property consistency verification exists to enforce: appending is
    /// fine, rewriting is not.
    #[test]
    fn an_append_only_tree_is_consistent_and_a_rewritten_one_is_not() {
        let records: Vec<&[u8]> = vec![b"a", b"b", b"c", b"d", b"e"];
        let full = build(&records);

        let old_size = 3u64;
        let new_size = 5u64;
        let old_root = tlog_tiles::tree_hash(old_size, &full).unwrap();
        let new_root = tlog_tiles::tree_hash(new_size, &full).unwrap();

        let proof = prove_tree(new_size, old_size, &full).unwrap();
        assert!(check_tree(&proof, new_size, new_root, old_size, old_root).is_ok());

        // A tree that replaced record 1 instead of appending to it.
        let rewritten: Vec<&[u8]> = vec![b"a", b"B", b"c", b"d", b"e"];
        let forked = build(&rewritten);
        let forked_root = tlog_tiles::tree_hash(new_size, &forked).unwrap();
        assert!(
            check_tree(&proof, new_size, forked_root, old_size, old_root).is_err(),
            "a rewritten tree must not verify against the old root"
        );
    }

    /// A log that replaces an entry without changing the tree size. Returning
    /// early on the size alone would wave exactly this through — there are no
    /// tiles to fetch, so the roots are the only evidence available.
    #[tokio::test]
    async fn the_same_size_with_a_different_root_is_a_rewrite() {
        let reader = HttpTileReader::new(
            "https://tiles.invalid",
            reqwest::Client::new(),
            Duration::from_millis(50),
            tokio::runtime::Handle::try_current().unwrap_or_else(|_| panic!("needs a runtime")),
            None,
        );
        let root = record_hash(b"one");
        let other = record_hash(b"another");

        assert!(matches!(
            verify_consistency(&reader, 256, root, 256, root),
            Verdict::Verified
        ));
        assert!(
            matches!(
                verify_consistency(&reader, 256, root, 256, other),
                Verdict::Mismatch(_)
            ),
            "a changed root at an unchanged size must not verify"
        );
    }

    /// A full tile is immutable and worth caching; a partial tile changes
    /// width as the log grows, so caching one accumulates an entry per width
    /// the log passed through and never serves a useful hit.
    #[tokio::test]
    async fn the_tile_cache_is_bounded_and_holds_only_full_tiles() {
        let reader = HttpTileReader::new(
            "https://tiles.invalid",
            reqwest::Client::new(),
            Duration::from_millis(50),
            tokio::runtime::Handle::try_current().unwrap_or_else(|_| panic!("needs a runtime")),
            None,
        );

        let full = vec![0u8; 256 * 32];
        for n in 0..(TILE_CACHE_CAPACITY as u64 * 3) {
            reader.save_tiles(
                &[Tile::new(TILE_HEIGHT, 0, n, 256, false)],
                std::slice::from_ref(&full),
            );
        }
        assert_eq!(reader.verified.lock().len(), TILE_CACHE_CAPACITY);

        for width in 1..50u32 {
            reader.save_tiles(
                &[Tile::new(TILE_HEIGHT, 0, 9_999, width, false)],
                &[vec![0u8; width as usize * 32]],
            );
        }
        assert_eq!(
            reader.verified.lock().len(),
            TILE_CACHE_CAPACITY,
            "partial tiles must not enter the cache at all"
        );
    }

    /// `node_hash` and `record_hash` use different prefixes; a verifier that
    /// confused them would accept a leaf as an interior node.
    #[test]
    fn leaf_and_interior_hashes_are_domain_separated() {
        let a = record_hash(b"x");
        let b = record_hash(b"y");
        assert_ne!(node_hash(a, b), record_hash(b"xy"));
    }
}
