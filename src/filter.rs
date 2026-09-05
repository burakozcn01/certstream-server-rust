//! Server-side subscription filters.
//!
//! A consumer watching one organisation's domains does not need the whole
//! firehose. Filtering here costs one match per *distinct* filter per
//! certificate, no matter how many subscribers share it — subscribers with
//! the same filter are served from one group, and the already-serialized
//! payload is handed to the group rather than re-rendered per client.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use parking_lot::Mutex;
use tokio::sync::broadcast;
use tracing::{debug, info};

use crate::models::{LeafCert, PreSerializedMessage};

/// Terms one subscriber may give per field. A filter is a watchlist, not a
/// query language; a subscriber that wants hundreds of domains wants the
/// unfiltered stream and its own index.
pub const MAX_TERMS_PER_FIELD: usize = 20;

/// Distinct filters the server will evaluate. Every certificate is matched
/// against every group, so this is the real cost ceiling of the feature.
pub const MAX_GROUPS: usize = 64;

/// Buffer depth of a group's channel. Filtered streams are a fraction of the
/// firehose, so a subscriber needs far less slack than on the main channel.
const GROUP_BUFFER: usize = 256;

#[derive(Debug, PartialEq, Eq)]
pub enum FilterError {
    TooManyTerms { field: &'static str, max: usize },
    EmptyTerm { field: &'static str },
    TooManyGroups,
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooManyTerms { field, max } => {
                write!(f, "at most {max} `{field}` terms are accepted")
            }
            Self::EmptyTerm { field } => write!(f, "`{field}` contains an empty term"),
            Self::TooManyGroups => write!(
                f,
                "the server is already evaluating {MAX_GROUPS} distinct filters"
            ),
        }
    }
}

/// What a subscriber asked to see.
///
/// Terms within a field are OR'd; the fields are AND'd. `domain=a.com,b.com`
/// with `issuer=let's encrypt` means "a.com or b.com, issued by Let's
/// Encrypt".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filter {
    /// Lowercased, dot-trimmed. Matches the name itself and any subdomain.
    domains: Vec<String>,
    /// Lowercased. Matched as a substring of the issuer CN and O, because
    /// the name a person knows ("let's encrypt") is the O while the CN is a
    /// rotating code ("R10").
    issuers: Vec<String>,
}

impl Filter {
    /// Build a filter from the raw query values. Returns `Ok(None)` when the
    /// subscriber asked for no filtering at all, which is the unfiltered
    /// firehose and not a filter group.
    pub fn parse(domain: Option<&str>, issuer: Option<&str>) -> Result<Option<Self>, FilterError> {
        let domains = parse_terms(domain, "domain", normalize_domain)?;
        let issuers = parse_terms(issuer, "issuer", |t| t.to_lowercase())?;

        if domains.is_empty() && issuers.is_empty() {
            return Ok(None);
        }
        Ok(Some(Self { domains, issuers }))
    }

    /// Canonical form. Two subscribers that asked for the same thing in a
    /// different order must land in the same group, or the server evaluates
    /// the same predicate twice.
    pub fn key(&self) -> String {
        let mut domains = self.domains.clone();
        let mut issuers = self.issuers.clone();
        domains.sort_unstable();
        domains.dedup();
        issuers.sort_unstable();
        issuers.dedup();
        format!("d={}|i={}", domains.join(","), issuers.join(","))
    }

    pub fn matches(&self, leaf: &LeafCert) -> bool {
        if !self.domains.is_empty()
            && !leaf
                .all_domains
                .iter()
                .any(|candidate| self.domains.iter().any(|f| domain_matches(f, candidate)))
        {
            return false;
        }

        if !self.issuers.is_empty() {
            let cn = leaf.issuer.cn.as_deref().unwrap_or("");
            let o = leaf.issuer.o.as_deref().unwrap_or("");
            if !self
                .issuers
                .iter()
                .any(|f| contains_ignore_ascii_case(cn, f) || contains_ignore_ascii_case(o, f))
            {
                return false;
            }
        }

        true
    }
}

fn parse_terms(
    raw: Option<&str>,
    field: &'static str,
    normalize: impl Fn(&str) -> String,
) -> Result<Vec<String>, FilterError> {
    let Some(raw) = raw else {
        return Ok(Vec::new());
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(Vec::new());
    }

    let mut terms = Vec::new();
    for part in raw.split(',') {
        let term = normalize(part.trim());
        if term.is_empty() {
            return Err(FilterError::EmptyTerm { field });
        }
        terms.push(term);
    }
    if terms.len() > MAX_TERMS_PER_FIELD {
        return Err(FilterError::TooManyTerms {
            field,
            max: MAX_TERMS_PER_FIELD,
        });
    }
    Ok(terms)
}

/// Lowercase, drop the root label's trailing dot, and drop a leading dot so
/// `.example.com` and `example.com` are the same watchlist entry.
fn normalize_domain(raw: &str) -> String {
    raw.trim()
        .trim_end_matches('.')
        .trim_start_matches('.')
        .to_lowercase()
}

/// True when `candidate` is `filter` or a subdomain of it.
///
/// The boundary check is the whole point: without the `.` test,
/// `notexample.com` ends with `example.com` and would match a filter for it.
/// A wildcard SAN is matched on the name it wildcards, so `*.example.com`
/// matches a filter for `example.com`.
fn domain_matches(filter: &str, candidate: &str) -> bool {
    let candidate = candidate
        .trim_end_matches('.')
        .strip_prefix("*.")
        .unwrap_or_else(|| candidate.trim_end_matches('.'));

    match candidate.len().cmp(&filter.len()) {
        std::cmp::Ordering::Equal => candidate.eq_ignore_ascii_case(filter),
        std::cmp::Ordering::Greater => {
            let (head, tail) = candidate.split_at(candidate.len() - filter.len());
            head.ends_with('.') && tail.eq_ignore_ascii_case(filter)
        }
        std::cmp::Ordering::Less => false,
    }
}

/// `needle` is already lowercase; `haystack` comes off the certificate.
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if haystack.len() < needle.len() {
        return false;
    }
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

struct FilterGroup {
    filter: Filter,
    tx: broadcast::Sender<Arc<PreSerializedMessage>>,
    /// Raw certificates the dispatcher missed upstream of this group.
    ///
    /// A subscriber's own channel shows it what *it* dropped. It cannot show
    /// what never reached the group, because the dispatcher lost those
    /// messages before it could match them. Counting here lets each
    /// subscriber notice the coverage gap and report it: the count is raw
    /// certificates, not matches, because once a message is gone nobody can
    /// say whether it would have matched.
    upstream_gap: Arc<AtomicU64>,
}

#[derive(Default)]
struct HubInner {
    groups: HashMap<String, FilterGroup>,
    dispatching: bool,
}

/// A seat in a filter group: the group's channel, plus a view of what the
/// dispatcher lost before it could match anything.
pub struct Subscription {
    pub rx: broadcast::Receiver<Arc<PreSerializedMessage>>,
    upstream_gap: Arc<AtomicU64>,
    seen_gap: u64,
}

impl Subscription {
    /// Certificates lost upstream since this was last called. Zero when the
    /// dispatcher has kept up, which is the normal case.
    pub fn take_upstream_gap(&mut self) -> u64 {
        let total = self.upstream_gap.load(Ordering::Relaxed);
        let delta = total.saturating_sub(self.seen_gap);
        self.seen_gap = total;
        delta
    }
}

/// Owns the filter groups and the single task that feeds them.
///
/// The dispatcher exists only while at least one group does. That matters
/// beyond tidiness: a permanently-subscribed dispatcher would keep the main
/// channel's `receiver_count()` above zero forever and defeat the idle-server
/// guard that skips serialization when nobody is listening.
pub struct FilterHub {
    inner: Mutex<HubInner>,
    /// Read on the ingest path to decide whether to retain the parsed leaf
    /// for matching. An atomic so that check costs nothing when unused.
    active: AtomicBool,
    source: broadcast::Sender<Arc<PreSerializedMessage>>,
}

impl FilterHub {
    pub fn new(source: broadcast::Sender<Arc<PreSerializedMessage>>) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(HubInner::default()),
            active: AtomicBool::new(false),
            source,
        })
    }

    /// Whether any filtered subscriber exists. When false the ingest path
    /// does not retain parsed leaves, so an unfiltered deployment pays
    /// nothing for this feature.
    #[inline]
    pub fn active(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    pub fn group_count(&self) -> usize {
        self.inner.lock().groups.len()
    }

    /// Join the group for `filter`, creating it (and the dispatcher) if this
    /// is the first subscriber for that exact filter.
    pub fn subscribe(self: &Arc<Self>, filter: Filter) -> Result<Subscription, FilterError> {
        let key = filter.key();
        let mut inner = self.inner.lock();

        if !inner.groups.contains_key(&key) && inner.groups.len() >= MAX_GROUPS {
            return Err(FilterError::TooManyGroups);
        }

        let group = inner.groups.entry(key.clone()).or_insert_with(|| {
            debug!(filter = %key, "opening filter group");
            FilterGroup {
                filter,
                tx: broadcast::channel(GROUP_BUFFER).0,
                upstream_gap: Arc::new(AtomicU64::new(0)),
            }
        });
        let subscription = Subscription {
            rx: group.tx.subscribe(),
            // Seeded with the current value so a subscriber is only ever told
            // about gaps that happened while it was connected.
            upstream_gap: Arc::clone(&group.upstream_gap),
            seen_gap: group.upstream_gap.load(Ordering::Relaxed),
        };

        self.active.store(true, Ordering::Relaxed);
        if !inner.dispatching {
            inner.dispatching = true;
            drop(inner);
            self.clone().spawn_dispatcher();
        }
        Ok(subscription)
    }

    fn spawn_dispatcher(self: Arc<Self>) {
        let mut rx = self.source.subscribe();
        tokio::spawn(async move {
            debug!("filter dispatcher started");
            loop {
                let msg = match rx.recv().await {
                    Ok(msg) => Some(msg),
                                    // The dispatcher is one subscriber among many and drops
                    // the same messages a slow client would.
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        metrics::counter!("certstream_filter_dispatch_lagged").increment(n);
                        // Charged to every group: the messages were lost
                        // before they could be matched, so any group might
                        // have wanted them.
                        for group in self.inner.lock().groups.values() {
                            group.upstream_gap.fetch_add(n, Ordering::Relaxed);
                        }
                        None
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                };

                let mut inner = self.inner.lock();
                inner.groups.retain(|key, group| {
                    let live = group.tx.receiver_count() > 0;
                    if !live {
                        debug!(filter = %key, "closing empty filter group");
                    }
                    live
                });

                if inner.groups.is_empty() {
                    inner.dispatching = false;
                    self.active.store(false, Ordering::Relaxed);
                    break;
                }

                if let Some(msg) = msg
                    // No leaf means the message was serialized before any
                    // filter existed. Nothing to match on, so it belongs to
                    // no group.
                    && let Some(leaf) = msg.leaf.as_deref()
                {
                    for group in inner.groups.values() {
                        if group.filter.matches(leaf) {
                            let _ = group.tx.send(Arc::clone(&msg));
                        }
                    }
                }

                metrics::gauge!("certstream_filter_groups").set(inner.groups.len() as f64);
            }
            metrics::gauge!("certstream_filter_groups").set(0.0);
            info!("filter dispatcher stopped");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Subject;

    fn leaf_with(domains: &[&str], issuer_cn: &str, issuer_o: &str) -> LeafCert {
        LeafCert {
            subject: Subject::default(),
            issuer: Subject {
                cn: Some(issuer_cn.to_string()),
                o: Some(issuer_o.to_string()),
                ..Default::default()
            },
            serial_number: String::new(),
            not_before: 0,
            not_after: 0,
            fingerprint: std::sync::Arc::from(""),
            sha1: String::new(),
            sha256: String::new(),
            sha256_raw: [0u8; 32],
            signature_algorithm: std::borrow::Cow::Borrowed("test"),
            is_ca: false,
            all_domains: domains.iter().map(|d| d.to_string()).collect(),
            as_der: None,
            extensions: Default::default(),
        }
    }

    fn filter(domain: Option<&str>, issuer: Option<&str>) -> Filter {
        Filter::parse(domain, issuer).unwrap().unwrap()
    }

    /// The rule the whole domain matcher exists for.
    #[test]
    fn a_filter_does_not_match_a_name_that_merely_ends_with_it() {
        let f = filter(Some("example.com"), None);
        assert!(!f.matches(&leaf_with(&["notexample.com"], "R10", "Let's Encrypt")));
        assert!(!f.matches(&leaf_with(&["myexample.com"], "R10", "Let's Encrypt")));
        assert!(!f.matches(&leaf_with(&["example.com.evil.net"], "R10", "X")));
    }

    #[test]
    fn a_filter_matches_the_name_itself_and_its_subdomains() {
        let f = filter(Some("example.com"), None);
        for name in [
            "example.com",
            "www.example.com",
            "a.b.c.example.com",
            "EXAMPLE.COM",
            "WWW.Example.Com",
            "example.com.",
        ] {
            assert!(f.matches(&leaf_with(&[name], "R10", "X")), "{name}");
        }
    }

    /// A wildcard SAN covers the name it wildcards, so it belongs to that
    /// name's watchlist.
    #[test]
    fn a_wildcard_san_matches_the_name_it_wildcards() {
        let f = filter(Some("example.com"), None);
        assert!(f.matches(&leaf_with(&["*.example.com"], "R10", "X")));

        let deeper = filter(Some("sub.example.com"), None);
        assert!(deeper.matches(&leaf_with(&["*.sub.example.com"], "R10", "X")));
        assert!(!deeper.matches(&leaf_with(&["*.other.example.com"], "R10", "X")));
    }

    #[test]
    fn any_matching_san_is_enough() {
        let f = filter(Some("example.com"), None);
        assert!(f.matches(&leaf_with(&["other.org", "www.example.com"], "R10", "X")));
        assert!(!f.matches(&leaf_with(&["other.org", "third.net"], "R10", "X")));
    }

    #[test]
    fn issuer_matches_the_cn_or_the_o() {
        let by_o = filter(None, Some("let's encrypt"));
        assert!(by_o.matches(&leaf_with(&["a.com"], "R10", "Let's Encrypt")));
        assert!(!by_o.matches(&leaf_with(&["a.com"], "WE1", "Google Trust Services")));

        let by_cn = filter(None, Some("r10"));
        assert!(by_cn.matches(&leaf_with(&["a.com"], "R10", "Let's Encrypt")));
        assert!(!by_cn.matches(&leaf_with(&["a.com"], "R11", "Let's Encrypt")));
    }

    #[test]
    fn domain_and_issuer_are_both_required_when_both_are_given() {
        let f = filter(Some("example.com"), Some("let's encrypt"));
        assert!(f.matches(&leaf_with(&["www.example.com"], "R10", "Let's Encrypt")));
        assert!(!f.matches(&leaf_with(&["www.example.com"], "WE1", "Google Trust Services")));
        assert!(!f.matches(&leaf_with(&["www.other.org"], "R10", "Let's Encrypt")));
    }

    #[test]
    fn terms_within_a_field_are_alternatives() {
        let f = filter(Some("example.com,other.org"), None);
        assert!(f.matches(&leaf_with(&["a.example.com"], "R10", "X")));
        assert!(f.matches(&leaf_with(&["b.other.org"], "R10", "X")));
        assert!(!f.matches(&leaf_with(&["c.third.net"], "R10", "X")));
    }

    /// Same request, different spelling: one group, one evaluation.
    #[test]
    fn equivalent_filters_share_a_key() {
        let a = filter(Some("b.com,a.com"), Some("Let's Encrypt"));
        let b = filter(Some("A.COM, b.com."), Some("let's encrypt"));
        assert_eq!(a.key(), b.key());

        let different = filter(Some("a.com"), None);
        assert_ne!(a.key(), different.key());
    }

    #[test]
    fn no_terms_means_no_filter() {
        assert_eq!(Filter::parse(None, None), Ok(None));
        assert_eq!(Filter::parse(Some(""), Some("  ")), Ok(None));
    }

    #[test]
    fn malformed_and_oversized_filters_are_rejected() {
        assert_eq!(
            Filter::parse(Some("a.com,,b.com"), None),
            Err(FilterError::EmptyTerm { field: "domain" })
        );

        let too_many = (0..=MAX_TERMS_PER_FIELD)
            .map(|i| format!("d{i}.com"))
            .collect::<Vec<_>>()
            .join(",");
        assert_eq!(
            Filter::parse(Some(&too_many), None),
            Err(FilterError::TooManyTerms {
                field: "domain",
                max: MAX_TERMS_PER_FIELD
            })
        );
    }

    #[tokio::test]
    async fn subscribers_with_the_same_filter_share_one_group() {
        let (tx, _rx) = broadcast::channel(16);
        let hub = FilterHub::new(tx);

        let _a = hub.subscribe(filter(Some("example.com"), None)).unwrap();
        let _b = hub.subscribe(filter(Some("EXAMPLE.COM"), None)).unwrap();
        assert_eq!(hub.group_count(), 1, "one predicate, one group");

        let _c = hub.subscribe(filter(Some("other.org"), None)).unwrap();
        assert_eq!(hub.group_count(), 2);
        assert!(hub.active());
    }

    /// A subscriber's own channel cannot show it what the dispatcher lost
    /// before matching. Without this counter that loss is invisible to it.
    #[tokio::test]
    async fn upstream_loss_is_reported_to_every_group_subscriber() {
        let (tx, _rx) = broadcast::channel(16);
        let hub = FilterHub::new(tx);

        let mut a = hub.subscribe(filter(Some("example.com"), None)).unwrap();
        let mut b = hub.subscribe(filter(Some("other.org"), None)).unwrap();
        assert_eq!(a.take_upstream_gap(), 0);
        assert_eq!(b.take_upstream_gap(), 0);

        // What the dispatcher does when it lags: every group might have
        // wanted the messages that were lost before matching.
        for group in hub.inner.lock().groups.values() {
            group.upstream_gap.fetch_add(7, Ordering::Relaxed);
        }

        assert_eq!(a.take_upstream_gap(), 7);
        assert_eq!(b.take_upstream_gap(), 7, "each group must be told");
        assert_eq!(a.take_upstream_gap(), 0, "a gap is reported once");
    }

    /// A subscriber must not be blamed for loss that happened before it
    /// connected.
    #[tokio::test]
    async fn a_new_subscriber_does_not_inherit_an_earlier_gap() {
        let (tx, _rx) = broadcast::channel(16);
        let hub = FilterHub::new(tx);

        let mut first = hub.subscribe(filter(Some("example.com"), None)).unwrap();
        for group in hub.inner.lock().groups.values() {
            group.upstream_gap.fetch_add(5, Ordering::Relaxed);
        }
        assert_eq!(first.take_upstream_gap(), 5);

        let mut late = hub.subscribe(filter(Some("example.com"), None)).unwrap();
        assert_eq!(late.take_upstream_gap(), 0);
    }

    #[tokio::test]
    async fn the_group_ceiling_is_enforced() {
        let (tx, _rx) = broadcast::channel(16);
        let hub = FilterHub::new(tx);

        let mut held = Vec::new();
        for i in 0..MAX_GROUPS {
            held.push(hub.subscribe(filter(Some(&format!("d{i}.com")), None)).unwrap());
        }
        let Err(err) = hub.subscribe(filter(Some("one.too.many.com"), None)) else {
            panic!("the group ceiling must reject a new predicate");
        };
        assert_eq!(err, FilterError::TooManyGroups);

        // An existing group still accepts more subscribers at the ceiling.
        assert!(hub.subscribe(filter(Some("d0.com"), None)).is_ok());
    }
}
