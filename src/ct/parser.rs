use base64::{engine::general_purpose::STANDARD, Engine};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::borrow::Cow;
use std::fmt::Write;
use std::net::IpAddr;
use x509_parser::der_parser::oid;
use x509_parser::extensions::ParsedExtension;
use x509_parser::oid_registry::Oid;
use x509_parser::prelude::*;

use crate::models::{ChainCert, DomainList, Extensions, LeafCert, Subject};

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
    let (update_type, cert_offset, is_precert): (Cow<'static, str>, usize, bool) = match entry_type
    {
        0 => (Cow::Borrowed("X509LogEntry"), 12, false),
        1 => (Cow::Borrowed("PrecertLogEntry"), 44, true),
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
    let mut leaf_cert = parse_certificate(cert_bytes, true)?;

    if is_precert {
        leaf_cert.extensions.ctl_poison_byte = true;
    }

    let chain = parse_chain(extra_data).unwrap_or_default();

    Some(ParsedEntry {
        update_type,
        leaf_cert,
        chain,
    })
}

fn parse_certificate(der_bytes: &[u8], include_der: bool) -> Option<LeafCert> {
    let (_, cert) = X509Certificate::from_der(der_bytes).ok()?;

    let mut subject = extract_name(cert.subject());
    let mut issuer = extract_name(cert.issuer());
    subject.build_aggregated();
    issuer.build_aggregated();

    let serial_number = format_serial_number(cert.serial.to_bytes_be());

    let sha1_hash = calculate_sha1(der_bytes);
    let sha256_hash = calculate_sha256(der_bytes);
    let fingerprint = sha1_hash.clone();

    let signature_algorithm = parse_signature_algorithm(&cert);
    let is_ca = cert.is_ca();

    let mut all_domains = DomainList::new();

    if let Some(ref cn) = subject.cn {
        if !cn.is_empty() && !is_ca {
            all_domains.push(cn.clone());
        }
    }

    let extensions = parse_extensions(&cert, &mut all_domains);

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
        sha1: sha1_hash,
        sha256: sha256_hash,
        signature_algorithm,
        is_ca,
        all_domains,
        as_der,
        extensions,
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
                sha1: leaf.sha1,
                sha256: leaf.sha256,
                signature_algorithm: leaf.signature_algorithm,
                is_ca: leaf.is_ca,
                as_der: leaf.as_der,
                extensions: leaf.extensions,
            });
        }

        offset += cert_len;
    }

    Some(chain)
}

fn extract_name(name: &X509Name) -> Subject {
    let mut subject = Subject::default();

    for rdn in name.iter() {
        for attr in rdn.iter() {
            let oid_str = attr.attr_type().to_id_string();
            if let Ok(value) = attr.attr_value().as_str() {
                match oid_str.as_str() {
                    "2.5.4.3" => subject.cn = Some(value.to_string()),
                    "2.5.4.6" => subject.c = Some(value.to_string()),
                    "2.5.4.7" => subject.l = Some(value.to_string()),
                    "2.5.4.8" => subject.st = Some(value.to_string()),
                    "2.5.4.10" => subject.o = Some(value.to_string()),
                    "2.5.4.11" => subject.ou = Some(value.to_string()),
                    "1.2.840.113549.1.9.1" => subject.email_address = Some(value.to_string()),
                    _ => {}
                }
            }
        }
    }

    subject
}

fn parse_extensions(cert: &X509Certificate, all_domains: &mut DomainList) -> Extensions {
    let mut ext = Extensions::default();
    let mut san_parts: Vec<String> = Vec::new();

    for extension in cert.extensions() {
        match extension.parsed_extension() {
            ParsedExtension::AuthorityKeyIdentifier(aki) => {
                if let Some(key_id) = &aki.key_identifier {
                    ext.authority_key_identifier = Some(format_key_id(key_id.0));
                }
            }
            ParsedExtension::SubjectKeyIdentifier(ski) => {
                ext.subject_key_identifier = Some(format_key_id(ski.0));
            }
            ParsedExtension::KeyUsage(ku) => {
                ext.key_usage = Some(key_usage_to_string(ku));
            }
            ParsedExtension::ExtendedKeyUsage(eku) => {
                ext.extended_key_usage = Some(extended_key_usage_to_string(eku));
            }
            ParsedExtension::BasicConstraints(bc) => {
                let ca_str = if bc.ca {
                    "CA:TRUE".to_string()
                } else {
                    "CA:FALSE".to_string()
                };
                ext.basic_constraints = Some(ca_str);
            }
            ParsedExtension::SubjectAlternativeName(san) => {
                for name in &san.general_names {
                    match name {
                        GeneralName::DNSName(dns) => {
                            san_parts.push(format!("DNS:{}", dns));
                            let domain = dns.to_string();
                            if !all_domains.contains(&domain) {
                                all_domains.push(domain);
                            }
                        }
                        GeneralName::RFC822Name(email) => {
                            san_parts.push(format!("email:{}", email));
                        }
                        GeneralName::IPAddress(ip_bytes) => {
                            if let Some(ip) = parse_ip_address(ip_bytes) {
                                san_parts.push(format!("IP Address:{}", ip));
                            }
                        }
                        _ => {}
                    }
                }
            }
            ParsedExtension::AuthorityInfoAccess(aia) => {
                let mut aia_parts: Vec<String> = Vec::new();
                for desc in &aia.accessdescs {
                    if let GeneralName::URI(uri) = &desc.access_location {
                        aia_parts.push(format!("URI:{}", uri));
                    }
                }
                if !aia_parts.is_empty() {
                    ext.authority_info_access = Some(aia_parts.join(", "));
                }
            }
            ParsedExtension::CertificatePolicies(policies) => {
                let mut policy_strs: Vec<String> = Vec::new();
                for policy in policies.iter() {
                    policy_strs.push(format!("Policy: {}\n", policy.policy_id));
                }
                if !policy_strs.is_empty() {
                    ext.certificate_policies = Some(policy_strs.concat());
                }
            }
            ParsedExtension::Unparsed => {
                if extension.oid == OID_X509_EXT_CT_POISON {
                    ext.ctl_poison_byte = true;
                }
            }
            _ => {}
        }
    }

    if !san_parts.is_empty() {
        ext.subject_alt_name = Some(san_parts.join(", "));
    }

    ext
}

fn parse_ip_address(bytes: &[u8]) -> Option<String> {
    match bytes.len() {
        4 => {
            let ip: IpAddr = std::net::Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]).into();
            Some(ip.to_string())
        }
        16 => {
            let arr: [u8; 16] = bytes.try_into().ok()?;
            let ip: IpAddr = std::net::Ipv6Addr::from(arr).into();
            Some(ip.to_string())
        }
        _ => None,
    }
}

fn format_key_id(key_id: &[u8]) -> String {
    let mut result = String::with_capacity(6 + key_id.len() * 3);
    result.push_str("keyid:");
    for (i, b) in key_id.iter().enumerate() {
        if i > 0 {
            result.push(':');
        }
        let _ = write!(result, "{:02x}", b);
    }
    result
}

fn format_serial_number(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut serial_number = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(serial_number, "{:02X}", b);
    }
    serial_number
}

fn calculate_sha1(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = String::with_capacity(32 * 3);
    for (i, b) in result.iter().enumerate() {
        if i > 0 {
            hash.push(':');
        }
        let _ = write!(hash, "{:02X}", b);
    }
    hash
}

fn calculate_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    let mut hash = String::with_capacity(32 * 3);
    for (i, b) in result.iter().enumerate() {
        if i > 0 {
            hash.push(':');
        }
        let _ = write!(hash, "{:02X}", b);
    }
    hash
}

fn parse_signature_algorithm(cert: &X509Certificate) -> String {
    let oid = cert.signature_algorithm.algorithm.to_id_string();
    match oid.as_str() {
        "1.2.840.113549.1.1.2" => "md2, rsa".to_string(),
        "1.2.840.113549.1.1.4" => "md5, rsa".to_string(),
        "1.2.840.113549.1.1.5" => "sha1, rsa".to_string(),
        "1.2.840.113549.1.1.11" => "sha256, rsa".to_string(),
        "1.2.840.113549.1.1.12" => "sha384, rsa".to_string(),
        "1.2.840.113549.1.1.13" => "sha512, rsa".to_string(),
        "1.2.840.113549.1.1.10" => "sha256, rsa-pss".to_string(),
        "1.2.840.10040.4.3" => "dsa, sha1".to_string(),
        "2.16.840.1.101.3.4.3.2" => "dsa, sha256".to_string(),
        "1.2.840.10045.4.1" => "ecdsa, sha1".to_string(),
        "1.2.840.10045.4.3.2" => "ecdsa, sha256".to_string(),
        "1.2.840.10045.4.3.3" => "ecdsa, sha384".to_string(),
        "1.2.840.10045.4.3.4" => "ecdsa, sha512".to_string(),
        "1.3.101.112" => "ed25519".to_string(),
        _ => "unknown".to_string(),
    }
}

fn key_usage_to_string(ku: &KeyUsage) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if ku.digital_signature() {
        parts.push("Digital Signature");
    }
    if ku.non_repudiation() {
        parts.push("Content Commitment");
    }
    if ku.key_encipherment() {
        parts.push("Key Encipherment");
    }
    if ku.data_encipherment() {
        parts.push("Data Encipherment");
    }
    if ku.key_agreement() {
        parts.push("Key Agreement");
    }
    if ku.key_cert_sign() {
        parts.push("Certificate Signing");
    }
    if ku.crl_sign() {
        parts.push("CRL Signing");
    }
    if ku.encipher_only() {
        parts.push("Encipher Only");
    }
    if ku.decipher_only() {
        parts.push("Decipher Only");
    }
    parts.join(", ")
}

fn extended_key_usage_to_string(eku: &ExtendedKeyUsage) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if eku.server_auth {
        parts.push("serverAuth");
    }
    if eku.client_auth {
        parts.push("clientAuth");
    }
    if eku.code_signing {
        parts.push("codeSigning");
    }
    if eku.email_protection {
        parts.push("emailProtection");
    }
    if eku.time_stamping {
        parts.push("timeStamping");
    }
    if eku.ocsp_signing {
        parts.push("OCSPSigning");
    }
    if eku.any {
        parts.push("anyExtendedKeyUsage");
    }
    parts.join(", ")
}

const OID_X509_EXT_CT_POISON: Oid<'static> = oid!(1.3.6 .1 .4 .1 .11129 .2 .4 .3);
