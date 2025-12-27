use base64::{engine::general_purpose::STANDARD, Engine};
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::collections::HashMap;
use x509_parser::prelude::*;

use crate::models::{ChainCert, DomainList, LeafCert};

pub struct ParsedEntry {
    pub update_type: Cow<'static, str>,
    pub leaf_cert: LeafCert,
    pub chain: Vec<ChainCert>,
}

pub fn parse_leaf_input(leaf_input: &str, extra_data: &str) -> Option<ParsedEntry> {
    let leaf_bytes = STANDARD.decode(leaf_input).ok()?;

    if leaf_bytes.len() < 15 {
        return None;
    }

    let entry_type = u16::from_be_bytes([leaf_bytes[10], leaf_bytes[11]]);
    let (update_type, cert_offset): (Cow<'static, str>, usize) = match entry_type {
        0 => (Cow::Borrowed("X509LogEntry"), 12),
        1 => (Cow::Borrowed("PrecertLogEntry"), 44),
        _ => return None,
    };

    if leaf_bytes.len() < cert_offset + 3 {
        return None;
    }

    let cert_data = &leaf_bytes[cert_offset..];
    let cert_len = u32::from_be_bytes([0, cert_data[0], cert_data[1], cert_data[2]]) as usize;

    if cert_data.len() < 3 + cert_len {
        return None;
    }

    let cert_bytes = &cert_data[3..3 + cert_len];
    let leaf_cert = parse_certificate(cert_bytes, true)?;
    let chain = parse_chain(extra_data).unwrap_or_default();

    Some(ParsedEntry {
        update_type,
        leaf_cert,
        chain,
    })
}

fn parse_certificate(der_bytes: &[u8], include_der: bool) -> Option<LeafCert> {
    let (_, cert) = X509Certificate::from_der(der_bytes).ok()?;

    let subject = extract_name(cert.subject());
    let issuer = extract_name(cert.issuer());

    let serial_bytes = cert.serial.to_bytes_be();
    let mut serial_number = String::with_capacity(serial_bytes.len() * 3);
    for (i, b) in serial_bytes.iter().enumerate() {
        if i > 0 {
            serial_number.push(':');
        }
        use std::fmt::Write;
        let _ = write!(serial_number, "{:02X}", b);
    }

    let fingerprint = {
        let mut hasher = Sha256::new();
        hasher.update(der_bytes);
        let result = hasher.finalize();
        let mut fp = String::with_capacity(7 + 32 * 3);
        fp.push_str("SHA256:");
        for (i, b) in result.iter().enumerate() {
            if i > 0 {
                fp.push(':');
            }
            use std::fmt::Write;
            let _ = write!(fp, "{:02X}", b);
        }
        fp
    };

    let mut all_domains = DomainList::new();

    if let Some(cn) = subject.get("CN") {
        if !cn.is_empty() {
            all_domains.push(cn.clone());
        }
    }

    if let Ok(Some(san)) = cert.subject_alternative_name() {
        for name in &san.value.general_names {
            if let GeneralName::DNSName(dns) = name {
                let domain = dns.to_string();
                if !all_domains.contains(&domain) {
                    all_domains.push(domain);
                }
            }
        }
    }

    let as_der = if include_der {
        Some(STANDARD.encode(der_bytes))
    } else {
        None
    };

    Some(LeafCert {
        subject,
        issuer,
        serial_number,
        not_before: cert.validity().not_before.timestamp(),
        not_after: cert.validity().not_after.timestamp(),
        fingerprint,
        all_domains,
        as_der,
        extensions: HashMap::new(),
    })
}

fn parse_chain(extra_data: &str) -> Option<Vec<ChainCert>> {
    let bytes = STANDARD.decode(extra_data).ok()?;
    if bytes.len() < 3 {
        return None;
    }

    let mut chain = Vec::with_capacity(4);
    let mut offset = 3;

    while offset + 3 < bytes.len() {
        let cert_len =
            u32::from_be_bytes([0, bytes[offset], bytes[offset + 1], bytes[offset + 2]]) as usize;
        offset += 3;

        if offset + cert_len > bytes.len() {
            break;
        }

        let cert_bytes = &bytes[offset..offset + cert_len];
        if let Some(leaf) = parse_certificate(cert_bytes, false) {
            chain.push(ChainCert {
                subject: leaf.subject,
                issuer: leaf.issuer,
                serial_number: leaf.serial_number,
                not_before: leaf.not_before,
                not_after: leaf.not_after,
                fingerprint: leaf.fingerprint,
                as_der: leaf.as_der,
            });
        }

        offset += cert_len;
    }

    Some(chain)
}

fn extract_name(name: &X509Name) -> HashMap<String, String> {
    let mut result = HashMap::with_capacity(6);

    for rdn in name.iter() {
        for attr in rdn.iter() {
            let key = match attr.attr_type().to_id_string().as_str() {
                "2.5.4.3" => "CN",
                "2.5.4.6" => "C",
                "2.5.4.7" => "L",
                "2.5.4.8" => "ST",
                "2.5.4.10" => "O",
                "2.5.4.11" => "OU",
                _ => continue,
            };

            if let Ok(value) = attr.attr_value().as_str() {
                result.insert(key.to_string(), value.to_string());
            }
        }
    }

    result
}
