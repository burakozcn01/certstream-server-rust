use bytes::Bytes;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::borrow::Cow;
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Subject {
    #[serde(rename = "C", skip_serializing_if = "Option::is_none")]
    pub c: Option<String>,
    #[serde(rename = "CN", skip_serializing_if = "Option::is_none")]
    pub cn: Option<String>,
    #[serde(rename = "L", skip_serializing_if = "Option::is_none")]
    pub l: Option<String>,
    #[serde(rename = "O", skip_serializing_if = "Option::is_none")]
    pub o: Option<String>,
    #[serde(rename = "OU", skip_serializing_if = "Option::is_none")]
    pub ou: Option<String>,
    #[serde(rename = "ST", skip_serializing_if = "Option::is_none")]
    pub st: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aggregated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email_address: Option<String>,
}

impl Subject {
    pub fn build_aggregated(&mut self) {
        let mut agg = String::new();
        if let Some(ref c) = self.c {
            agg.push_str("/C=");
            agg.push_str(c);
        }
        if let Some(ref cn) = self.cn {
            agg.push_str("/CN=");
            agg.push_str(cn);
        }
        if let Some(ref l) = self.l {
            agg.push_str("/L=");
            agg.push_str(l);
        }
        if let Some(ref o) = self.o {
            agg.push_str("/O=");
            agg.push_str(o);
        }
        if let Some(ref ou) = self.ou {
            agg.push_str("/OU=");
            agg.push_str(ou);
        }
        if let Some(ref st) = self.st {
            agg.push_str("/ST=");
            agg.push_str(st);
        }
        self.aggregated = Some(agg);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Extensions {
    #[serde(rename = "authorityInfoAccess", skip_serializing_if = "Option::is_none")]
    pub authority_info_access: Option<String>,
    #[serde(rename = "authorityKeyIdentifier", skip_serializing_if = "Option::is_none")]
    pub authority_key_identifier: Option<String>,
    #[serde(rename = "basicConstraints", skip_serializing_if = "Option::is_none")]
    pub basic_constraints: Option<String>,
    #[serde(rename = "certificatePolicies", skip_serializing_if = "Option::is_none")]
    pub certificate_policies: Option<String>,
    #[serde(rename = "ctlSignedCertificateTimestamp", skip_serializing_if = "Option::is_none")]
    pub ctl_signed_certificate_timestamp: Option<String>,
    #[serde(rename = "extendedKeyUsage", skip_serializing_if = "Option::is_none")]
    pub extended_key_usage: Option<String>,
    #[serde(rename = "keyUsage", skip_serializing_if = "Option::is_none")]
    pub key_usage: Option<String>,
    #[serde(rename = "subjectAltName", skip_serializing_if = "Option::is_none")]
    pub subject_alt_name: Option<String>,
    #[serde(rename = "subjectKeyIdentifier", skip_serializing_if = "Option::is_none")]
    pub subject_key_identifier: Option<String>,
    #[serde(rename = "ctlPoisonByte", skip_serializing_if = "is_false")]
    pub ctl_poison_byte: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeafCert {
    pub subject: Subject,
    pub issuer: Subject,
    pub serial_number: String,
    pub not_before: i64,
    pub not_after: i64,
    pub fingerprint: String,
    pub sha1: String,
    pub sha256: String,
    pub signature_algorithm: String,
    pub is_ca: bool,
    pub all_domains: DomainList,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub as_der: Option<String>,
    pub extensions: Extensions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainCert {
    pub subject: Subject,
    pub issuer: Subject,
    pub serial_number: String,
    pub not_before: i64,
    pub not_after: i64,
    pub fingerprint: String,
    pub sha1: String,
    pub sha256: String,
    pub signature_algorithm: String,
    pub is_ca: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub as_der: Option<String>,
    pub extensions: Extensions,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub name: Arc<str>,
    pub url: Arc<str>,
    #[serde(skip_serializing)]
    #[allow(dead_code)]
    pub operator: Arc<str>,
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
    subject: &'a Subject,
    issuer: &'a Subject,
    serial_number: &'a str,
    not_before: i64,
    not_after: i64,
    fingerprint: &'a str,
    sha1: &'a str,
    sha256: &'a str,
    signature_algorithm: &'a str,
    is_ca: bool,
    all_domains: &'a DomainList,
    extensions: &'a Extensions,
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
                    sha1: &self.data.leaf_cert.sha1,
                    sha256: &self.data.leaf_cert.sha256,
                    signature_algorithm: &self.data.leaf_cert.signature_algorithm,
                    is_ca: self.data.leaf_cert.is_ca,
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
