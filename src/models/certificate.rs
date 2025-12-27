use bytes::Bytes;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

pub type DomainList = SmallVec<[String; 4]>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateMessage {
    pub message_type: Cow<'static, str>,
    pub data: CertificateData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertificateData {
    pub update_type: Cow<'static, str>,
    pub leaf_cert: LeafCert,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub chain: Option<Vec<ChainCert>>,
    pub cert_index: u64,
    pub seen: f64,
    pub source: Arc<Source>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeafCert {
    pub subject: HashMap<String, String>,
    pub issuer: HashMap<String, String>,
    pub serial_number: String,
    pub not_before: i64,
    pub not_after: i64,
    pub fingerprint: String,
    pub all_domains: DomainList,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub as_der: Option<String>,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    pub extensions: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainCert {
    pub subject: HashMap<String, String>,
    pub issuer: HashMap<String, String>,
    pub serial_number: String,
    pub not_before: i64,
    pub not_after: i64,
    pub fingerprint: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub as_der: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub name: Arc<str>,
    pub url: Arc<str>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainsOnlyMessage {
    pub message_type: Cow<'static, str>,
    pub data: DomainsOnlyData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DomainsOnlyData {
    pub update_type: Cow<'static, str>,
    pub all_domains: DomainList,
    pub seen: f64,
    pub source: Arc<Source>,
}

#[derive(Debug, Clone)]
pub struct PreSerializedMessage {
    pub full: Bytes,
    pub lite: Bytes,
    pub domains_only: Bytes,
}

impl PreSerializedMessage {
    pub fn from_certificate(msg: &CertificateMessage) -> Option<Self> {
        let full = serde_json::to_vec(msg).ok()?;

        let lite_msg = msg.to_lite();
        let lite = serde_json::to_vec(&lite_msg).ok()?;

        let domains_msg = msg.to_domains_only();
        let domains_only = serde_json::to_vec(&domains_msg).ok()?;

        Some(Self {
            full: Bytes::from(full),
            lite: Bytes::from(lite),
            domains_only: Bytes::from(domains_only),
        })
    }
}

#[derive(Debug, Clone, Serialize)]
struct LiteMessage<'a> {
    message_type: &'a Cow<'static, str>,
    data: LiteData<'a>,
}

#[derive(Debug, Clone, Serialize)]
struct LiteData<'a> {
    update_type: &'a Cow<'static, str>,
    leaf_cert: LiteLeafCert<'a>,
    cert_index: u64,
    seen: f64,
    source: &'a Arc<Source>,
}

#[derive(Debug, Clone, Serialize)]
struct LiteLeafCert<'a> {
    subject: &'a HashMap<String, String>,
    issuer: &'a HashMap<String, String>,
    serial_number: &'a str,
    not_before: i64,
    not_after: i64,
    fingerprint: &'a str,
    all_domains: &'a DomainList,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    extensions: &'a HashMap<String, serde_json::Value>,
}

impl CertificateMessage {
    pub fn to_domains_only(&self) -> DomainsOnlyMessage {
        DomainsOnlyMessage {
            message_type: Cow::Borrowed("certificate_update"),
            data: DomainsOnlyData {
                update_type: self.data.update_type.clone(),
                all_domains: self.data.leaf_cert.all_domains.clone(),
                seen: self.data.seen,
                source: Arc::clone(&self.data.source),
            },
        }
    }

    fn to_lite(&self) -> LiteMessage<'_> {
        LiteMessage {
            message_type: &self.message_type,
            data: LiteData {
                update_type: &self.data.update_type,
                leaf_cert: LiteLeafCert {
                    subject: &self.data.leaf_cert.subject,
                    issuer: &self.data.leaf_cert.issuer,
                    serial_number: &self.data.leaf_cert.serial_number,
                    not_before: self.data.leaf_cert.not_before,
                    not_after: self.data.leaf_cert.not_after,
                    fingerprint: &self.data.leaf_cert.fingerprint,
                    all_domains: &self.data.leaf_cert.all_domains,
                    extensions: &self.data.leaf_cert.extensions,
                },
                cert_index: self.data.cert_index,
                seen: self.data.seen,
                source: &self.data.source,
            },
        }
    }

    #[inline]
    pub fn pre_serialize(self) -> Option<Arc<PreSerializedMessage>> {
        PreSerializedMessage::from_certificate(&self).map(Arc::new)
    }
}
