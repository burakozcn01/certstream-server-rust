//! Sunlight's optional names-tiles extension: the names on the certificates
//! in a data tile, as compact JSON lines.
//!
//! For a deployment that only wants domain names this is a much cheaper way
//! to read a log — measured at roughly 3% of the data tile's bytes on
//! Geomys' Tuscolo (6.6 KB against 226 KB for the same tile), with no X.509
//! parsing at all.
//!
//! The trade is stated plainly by the extension itself: names tiles are
//! unauthenticated. They cannot be checked for inclusion in a signed tree
//! head, so nothing read this way carries the guarantees the data-tile path
//! does. That boundary is carried through to the wire — entries collected
//! here are published as `dns_entries_unauthenticated`, never as
//! `dns_entries` — so a consumer can never mistake one source for the other.
//!
//! <https://github.com/FiloSottile/sunlight/blob/main/names-tiles.md>

use bytes::Bytes;
use serde::Deserialize;
use smallvec::SmallVec;

use crate::models::DomainList;

/// One line of a names tile. Only the names are read; the rest of the
/// `TrimmedEntry` is the subject, which this mode does not publish.
#[derive(Debug, Deserialize)]
struct TrimmedEntry {
    #[serde(rename = "Timestamp")]
    timestamp: u64,
    #[serde(rename = "DNS")]
    #[serde(default)]
    dns: Vec<String>,
    #[serde(rename = "Subject")]
    #[serde(default)]
    subject: Option<TrimmedSubject>,
}

#[derive(Debug, Deserialize)]
struct TrimmedSubject {
    #[serde(rename = "CommonName")]
    #[serde(default)]
    common_name: Option<String>,
}

/// An entry as this mode publishes it: when the log issued it, and the names.
#[derive(Debug, PartialEq, Eq)]
pub struct NamesEntry {
    pub submission_timestamp: u64,
    pub domains: DomainList,
}

/// Where a names tile lives. Mirrors the data-tile path, per the extension.
pub fn names_tile_url(base_url: &str, tile_index: u64, partial_width: u64) -> String {
    let base = base_url.trim_end_matches('/');
    let path = super::static_ct::encode_tile_path(tile_index);
    if partial_width > 0 && partial_width < 256 {
        format!("{base}/tile/names/{path}.p/{partial_width}")
    } else {
        format!("{base}/tile/names/{path}")
    }
}

/// Parse a names tile body into one slot per line, in tile order.
///
/// The position of a slot is the entry's offset within the tile, and the
/// caller derives log indexes from it — so a line that yields nothing must
/// still occupy its slot as `None`. Compacting the vector instead would shift
/// every later entry down and make the caller's resume offset skip real
/// names.
///
/// A line that does not parse yields `None` rather than failing the tile: a
/// log that adds a field must not stop a monitor, and the extension is
/// explicitly a convenience format rather than a signed one.
pub fn parse_names_tile(body: Bytes) -> Vec<Option<NamesEntry>> {
    // Servers send these gzipped; `decompress_tile` passes non-gzip bodies
    // through, so this handles a log that serves them plain.
    let decompressed = super::static_ct::decompress_tile(&body);

    let mut out = Vec::new();
    for line in decompressed.split(|b| *b == b'\n') {
        // A trailing newline produces one empty trailing element, which is not
        // an entry slot.
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_slice::<TrimmedEntry>(line) else {
            out.push(None);
            continue;
        };

        // The CN is frequently also a SAN, but not always, and a monitor that
        // dropped a CN-only name would miss exactly the certificates worth
        // noticing.
        let mut domains: DomainList = SmallVec::new();
        if let Some(cn) = entry.subject.and_then(|s| s.common_name)
            && !cn.is_empty()
            && !entry.dns.iter().any(|d| d == &cn)
        {
            domains.push(cn);
        }
        domains.extend(entry.dns);

        if domains.is_empty() {
            out.push(None);
            continue;
        }
        out.push(Some(NamesEntry {
            submission_timestamp: entry.timestamp,
            domains,
        }));
    }
    out
}

/// Dedup key for an entry read this way.
///
/// The data-tile path dedups on the certificate's SHA-256. Names tiles carry
/// no certificate, so this hashes what they do carry. That dedups repeats of
/// the same entry, and does *not* collapse the same certificate seen in two
/// logs — their SCT timestamps differ. Cross-log dedup is one of the things
/// this mode gives up.
pub fn dedup_key(entry: &NamesEntry) -> [u8; 32] {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(entry.submission_timestamp.to_be_bytes());
    for domain in &entry.domains {
        hasher.update(domain.as_bytes());
        hasher.update([0]);
    }
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(lines: &[&str]) -> Bytes {
        Bytes::from(lines.join("\n"))
    }

    fn entries(body: Bytes) -> Vec<Option<NamesEntry>> {
        parse_names_tile(body)
    }

    fn names_at(slots: &[Option<NamesEntry>], i: usize) -> &[String] {
        slots[i].as_ref().expect("slot holds an entry").domains.as_slice()
    }

    #[test]
    fn a_names_tile_parses_into_one_slot_per_line() {
        let body = tile(&[
            r#"{"Timestamp":1747699628116,"Subject":{"CommonName":"a.example"},"DNS":["a.example","www.a.example"]}"#,
            r#"{"Timestamp":1747699628999,"Subject":{"CommonName":"b.example"},"DNS":["b.example"]}"#,
        ]);

        let slots = entries(body);
        assert_eq!(slots.len(), 2);
        assert_eq!(
            slots[0].as_ref().unwrap().submission_timestamp,
            1_747_699_628_116
        );
        assert_eq!(names_at(&slots, 0), ["a.example", "www.a.example"]);
    }

    /// A slot the parser could not use still occupies its position. Compacting
    /// it away shifts every later entry down, and the watcher's resume offset
    /// — which counts real log indexes — then skips a name it should deliver.
    #[test]
    fn an_unusable_line_keeps_its_slot_so_offsets_stay_aligned() {
        let body = tile(&[
            r#"{"Timestamp":1,"DNS":[]}"#,
            r#"{"Timestamp":2,"DNS":["wanted.example"]}"#,
        ]);

        let slots = entries(body);
        assert_eq!(slots.len(), 2, "both lines must occupy a slot");
        assert!(slots[0].is_none());
        assert_eq!(names_at(&slots, 1), ["wanted.example"]);

        // What the watcher does when resuming one entry into the tile.
        let resumed: Vec<&str> = slots
            .iter()
            .skip(1)
            .flatten()
            .flat_map(|e| e.domains.iter().map(String::as_str))
            .collect();
        assert_eq!(
            resumed,
            ["wanted.example"],
            "resuming at offset 1 must deliver the entry at index 1"
        );
    }

    /// A CN that is not also a SAN is exactly the certificate a monitor wants
    /// to see, so it must not be dropped.
    #[test]
    fn a_common_name_missing_from_the_sans_is_kept() {
        let body = tile(&[
            r#"{"Timestamp":1,"Subject":{"CommonName":"only-cn.example"},"DNS":["other.example"]}"#,
        ]);
        assert_eq!(
            names_at(&entries(body), 0),
            ["only-cn.example", "other.example"]
        );
    }

    #[test]
    fn a_common_name_already_in_the_sans_is_not_duplicated() {
        let body = tile(&[
            r#"{"Timestamp":1,"Subject":{"CommonName":"a.example"},"DNS":["a.example","b.example"]}"#,
        ]);
        assert_eq!(names_at(&entries(body), 0), ["a.example", "b.example"]);
    }

    /// A log adding a field, or one bad line, must not cost the whole tile.
    #[test]
    fn unparseable_lines_yield_empty_slots_not_a_failed_tile() {
        let body = tile(&[
            r#"{"Timestamp":1,"DNS":["good.example"]}"#,
            "{not json",
            r#"{"Timestamp":2,"DNS":[],"Subject":{}}"#,
            r#"{"Timestamp":3,"DNS":["also-good.example"],"FutureField":42}"#,
        ]);

        let slots = entries(body);
        assert_eq!(slots.len(), 4, "{slots:?}");
        assert_eq!(names_at(&slots, 0), ["good.example"]);
        assert!(slots[1].is_none());
        assert!(slots[2].is_none());
        assert_eq!(names_at(&slots, 3), ["also-good.example"]);
    }

    #[test]
    fn names_tile_paths_mirror_data_tile_paths() {
        assert_eq!(
            names_tile_url("https://log.example/2026h2/", 1234, 0),
            "https://log.example/2026h2/tile/names/x001/234"
        );
        assert_eq!(
            names_tile_url("https://log.example", 7, 42),
            "https://log.example/tile/names/007.p/42"
        );
    }

    #[test]
    fn the_dedup_key_separates_entries_that_differ() {
        let base = NamesEntry {
            submission_timestamp: 1,
            domains: SmallVec::from_vec(vec!["a.example".into()]),
        };
        let same = NamesEntry {
            submission_timestamp: 1,
            domains: SmallVec::from_vec(vec!["a.example".into()]),
        };
        let later = NamesEntry {
            submission_timestamp: 2,
            domains: SmallVec::from_vec(vec!["a.example".into()]),
        };
        let other = NamesEntry {
            submission_timestamp: 1,
            domains: SmallVec::from_vec(vec!["b.example".into()]),
        };

        assert_eq!(dedup_key(&base), dedup_key(&same));
        assert_ne!(dedup_key(&base), dedup_key(&later));
        assert_ne!(dedup_key(&base), dedup_key(&other));
    }

    /// The separator matters: without it `["ab","c"]` and `["a","bc"]` hash
    /// the same, and one would silently swallow the other.
    #[test]
    fn the_dedup_key_is_not_confused_by_name_boundaries() {
        let left = NamesEntry {
            submission_timestamp: 1,
            domains: SmallVec::from_vec(vec!["ab".into(), "c".into()]),
        };
        let right = NamesEntry {
            submission_timestamp: 1,
            domains: SmallVec::from_vec(vec!["a".into(), "bc".into()]),
        };
        assert_ne!(dedup_key(&left), dedup_key(&right));
    }
}
