//! Pure-Rust XML DOM parser, W3C Exclusive XML Canonicalization (`xml-exc-c14n#`),
//! X.509 certificate SPKI extraction, and strict SAML 2.0 XML-DSig verifier.
//!
//! Security properties:
//! - Rejects `<!DOCTYPE`, `<!ENTITY`, and custom entity references (anti-XXE).
//! - Enforces element depth and count bounds.
//! - Rejects duplicate `ID` attributes anywhere in the document (anti-XSW).
//! - Rejects `<EncryptedAssertion>`, nested `<Assertion>` wrappers, or multiple `<Assertion>`s.
//! - Requires a valid enveloped `<ds:Signature>` directly on root `<Response>` (covering
//!   `#<response_id>`) and/or directly on the sole child `<Assertion>` (covering `#<assertion_id>`).
//! - Verifies signatures exclusively against caller-pinned IdP X.509 certificates, never
//!   trusting unpinned `<KeyInfo>` embedded in untrusted assertions.

use crate::error::SamlError;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use der::{Reader as _, SliceReader, Tag, TagNumber};
use p256::ecdsa::signature::Verifier as _;
use p256::pkcs8::DecodePublicKey as _;
use quick_xml::events::Event;
use quick_xml::Reader;
use rsa::pkcs1v15::{Signature as RsaSignature, VerifyingKey as RsaVerifyingKey};
use rsa::RsaPublicKey;
use sha2::{Digest, Sha256, Sha512};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use subtle::ConstantTimeEq;

pub(crate) const XMLNS_XML: &str = "http://www.w3.org/XML/1998/namespace";
pub(crate) const XMLNS_SAML_PROTOCOL: &str = "urn:oasis:names:tc:SAML:2.0:protocol";
pub(crate) const XMLNS_SAML_ASSERTION: &str = "urn:oasis:names:tc:SAML:2.0:assertion";
pub(crate) const XMLNS_DSIG: &str = "http://www.w3.org/2000/09/xmldsig#";
pub(crate) const XMLNS_EXC_C14N: &str = "http://www.w3.org/2001/10/xml-exc-c14n#";

pub(crate) const ALG_ENVELOPED: &str = "http://www.w3.org/2000/09/xmldsig#enveloped-signature";
pub(crate) const ALG_EXC_C14N: &str = "http://www.w3.org/2001/10/xml-exc-c14n#";
pub(crate) const ALG_EXC_C14N_COMMENTS: &str =
    "http://www.w3.org/2001/10/xml-exc-c14n#WithComments";
pub(crate) const ALG_DIGEST_SHA256: &str = "http://www.w3.org/2001/04/xmlenc#sha256";
pub(crate) const ALG_DIGEST_SHA512: &str = "http://www.w3.org/2001/04/xmlenc#sha512";
pub(crate) const ALG_SIG_RSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha256";
pub(crate) const ALG_SIG_RSA_SHA512: &str = "http://www.w3.org/2001/04/xmldsig-more#rsa-sha512";
pub(crate) const ALG_SIG_ECDSA_SHA256: &str = "http://www.w3.org/2001/04/xmldsig-more#ecdsa-sha256";

const MAX_XML_DEPTH: usize = 64;
const MAX_XML_ELEMENTS: usize = 10_000;

/// Parsed XML attribute (excluding `xmlns` declarations).
#[derive(Debug, Clone)]
pub(crate) struct XmlAttr {
    pub(crate) namespace_uri: String,
    pub(crate) prefix: String,
    pub(crate) local_name: String,
    pub(crate) qname: String,
    pub(crate) value: String,
}

/// Child node inside an [`XmlElement`].
#[derive(Debug, Clone)]
pub(crate) enum XmlChild {
    Element(XmlElement),
    Text(String),
}

/// Immutable XML element node with resolved namespaces and attributes.
#[derive(Debug, Clone)]
pub(crate) struct XmlElement {
    pub(crate) qname: String,
    pub(crate) prefix: String,
    pub(crate) local_name: String,
    pub(crate) namespace_uri: String,
    pub(crate) ns_decls: BTreeMap<String, String>,
    pub(crate) attrs: Vec<XmlAttr>,
    pub(crate) children: Vec<XmlChild>,
}

impl XmlElement {
    pub(crate) fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|a| a.local_name == name && a.namespace_uri.is_empty())
            .map(|a| a.value.as_str())
    }

    pub(crate) fn child_elements(&self) -> impl Iterator<Item = &XmlElement> {
        self.children.iter().filter_map(|c| match c {
            XmlChild::Element(e) => Some(e),
            XmlChild::Text(_) => None,
        })
    }

    pub(crate) fn find_direct_child(&self, ns: &str, local: &str) -> Option<&XmlElement> {
        self.child_elements()
            .find(|e| e.local_name == local && e.namespace_uri == ns)
    }

    pub(crate) fn text_content(&self) -> String {
        let mut out = String::new();
        for child in &self.children {
            match child {
                XmlChild::Text(t) => out.push_str(t),
                XmlChild::Element(e) => out.push_str(&e.text_content()),
            }
        }
        out
    }
}

/// Parses an XML string strictly without DTDs or custom entities.
pub(crate) fn parse_xml(xml: &str) -> Result<XmlElement, SamlError> {
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<(XmlElement, BTreeMap<String, String>)> = Vec::new();
    let mut initial_ns = BTreeMap::new();
    initial_ns.insert("xml".to_string(), XMLNS_XML.to_string());
    let mut root: Option<XmlElement> = None;
    let mut seen_ids: HashSet<String> = HashSet::new();
    let mut element_count: usize = 0;

    loop {
        match reader.read_event() {
            Ok(Event::Decl(_)) | Ok(Event::Comment(_)) => {}
            Ok(Event::DocType(_)) => {
                return Err(SamlError::SignatureWrapping(
                    "DOCTYPE / DTD declarations are forbidden in SAML XML".to_string(),
                ));
            }
            Ok(Event::PI(_)) => {
                return Err(SamlError::SignatureWrapping(
                    "processing instructions are forbidden in SAML XML".to_string(),
                ));
            }
            Ok(Event::Start(e)) => {
                element_count += 1;
                if element_count > MAX_XML_ELEMENTS {
                    return Err(SamlError::SignatureWrapping(
                        "XML element limit exceeded".to_string(),
                    ));
                }
                if stack.len() >= MAX_XML_DEPTH {
                    return Err(SamlError::SignatureWrapping(
                        "XML nesting depth limit exceeded".to_string(),
                    ));
                }
                let parent_ns = stack.last().map(|(_, ns)| ns).unwrap_or(&initial_ns);
                let (elem, scope) = build_element(&e, parent_ns, &mut seen_ids)?;
                stack.push((elem, scope));
            }
            Ok(Event::Empty(e)) => {
                element_count += 1;
                if element_count > MAX_XML_ELEMENTS {
                    return Err(SamlError::SignatureWrapping(
                        "XML element limit exceeded".to_string(),
                    ));
                }
                let parent_ns = stack.last().map(|(_, ns)| ns).unwrap_or(&initial_ns);
                let (elem, _) = build_element(&e, parent_ns, &mut seen_ids)?;
                if let Some((parent, _)) = stack.last_mut() {
                    parent.children.push(XmlChild::Element(elem));
                } else if root.is_none() {
                    root = Some(elem);
                } else {
                    return Err(SamlError::InvalidMetadata(
                        "multiple XML root elements".to_string(),
                    ));
                }
            }
            Ok(Event::End(e)) => {
                let Some((elem, _)) = stack.pop() else {
                    return Err(SamlError::InvalidMetadata(
                        "unexpected XML end tag".to_string(),
                    ));
                };
                let end_name = e.name();
                let end_qname = std::str::from_utf8(end_name.as_ref()).map_err(|err| {
                    SamlError::InvalidMetadata(format!("invalid UTF-8 tag: {err}"))
                })?;
                if elem.qname != end_qname {
                    return Err(SamlError::InvalidMetadata(format!(
                        "mismatched XML end tag '</{end_qname}>' for '<{}>'",
                        elem.qname
                    )));
                }
                if let Some((parent, _)) = stack.last_mut() {
                    parent.children.push(XmlChild::Element(elem));
                } else if root.is_none() {
                    root = Some(elem);
                } else {
                    return Err(SamlError::InvalidMetadata(
                        "multiple XML root elements".to_string(),
                    ));
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.decode().map_err(|err| {
                    SamlError::InvalidMetadata(format!("invalid XML text: {err}"))
                })?;
                if let Some((parent, _)) = stack.last_mut() {
                    push_text(&mut parent.children, &text);
                }
            }
            Ok(Event::CData(c)) => {
                let text = c.decode().map_err(|err| {
                    SamlError::InvalidMetadata(format!("invalid XML CDATA: {err}"))
                })?;
                if let Some((parent, _)) = stack.last_mut() {
                    push_text(&mut parent.children, &text);
                }
            }
            Ok(Event::GeneralRef(r)) => {
                let name = r.decode().map_err(|err| {
                    SamlError::InvalidMetadata(format!("invalid XML entity reference: {err}"))
                })?;
                let resolved = match name.as_ref() {
                    "lt" => "<".to_string(),
                    "gt" => ">".to_string(),
                    "amp" => "&".to_string(),
                    "apos" => "'".to_string(),
                    "quot" => "\"".to_string(),
                    _ => match r.resolve_char_ref().map_err(|err| {
                        SamlError::SignatureWrapping(format!("invalid char ref '&{name};': {err}"))
                    })? {
                        Some(ch) => ch.to_string(),
                        None => {
                            return Err(SamlError::SignatureWrapping(format!(
                                "custom XML entity reference '&{name};' is forbidden"
                            )));
                        }
                    },
                };
                if let Some((parent, _)) = stack.last_mut() {
                    push_text(&mut parent.children, &resolved);
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => {
                return Err(SamlError::InvalidMetadata(format!("malformed XML: {err}")));
            }
        }
    }

    if !stack.is_empty() {
        return Err(SamlError::InvalidMetadata("unclosed XML tags".to_string()));
    }
    root.ok_or_else(|| SamlError::InvalidMetadata("empty XML document".to_string()))
}

fn push_text(children: &mut Vec<XmlChild>, text: &str) {
    if let Some(XmlChild::Text(existing)) = children.last_mut() {
        existing.push_str(text);
    } else {
        children.push(XmlChild::Text(text.to_string()));
    }
}

fn split_qname(qname: &str) -> (&str, &str) {
    match qname.split_once(':') {
        Some((prefix, local)) => (prefix, local),
        None => ("", qname),
    }
}

fn build_element(
    start: &quick_xml::events::BytesStart<'_>,
    parent_ns: &BTreeMap<String, String>,
    seen_ids: &mut HashSet<String>,
) -> Result<(XmlElement, BTreeMap<String, String>), SamlError> {
    let qname = std::str::from_utf8(start.name().as_ref())
        .map_err(|e| SamlError::InvalidMetadata(format!("invalid UTF-8 element name: {e}")))?
        .to_string();
    let (prefix_str, local_str) = split_qname(&qname);
    let prefix = prefix_str.to_string();
    let local_name = local_str.to_string();

    let mut scope = parent_ns.clone();
    let mut ns_decls = BTreeMap::new();
    let mut raw_attrs = Vec::new();

    for attr_res in start.attributes() {
        let attr = attr_res
            .map_err(|e| SamlError::InvalidMetadata(format!("malformed XML attribute: {e}")))?;
        let key = std::str::from_utf8(attr.key.as_ref())
            .map_err(|e| SamlError::InvalidMetadata(format!("invalid UTF-8 attribute key: {e}")))?
            .to_string();
        let val = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map_err(|e| SamlError::InvalidMetadata(format!("invalid XML attribute value: {e}")))?
            .into_owned();

        if key == "xmlns" {
            ns_decls.insert(String::new(), val.clone());
            scope.insert(String::new(), val);
        } else if let Some(ns_prefix) = key.strip_prefix("xmlns:") {
            ns_decls.insert(ns_prefix.to_string(), val.clone());
            scope.insert(ns_prefix.to_string(), val);
        } else {
            raw_attrs.push((key, val));
        }
    }

    let namespace_uri = if prefix.is_empty() {
        scope.get("").cloned().unwrap_or_default()
    } else {
        scope.get(&prefix).cloned().ok_or_else(|| {
            SamlError::InvalidMetadata(format!("unbound XML namespace prefix '{prefix}'"))
        })?
    };

    let mut seen_attr_keys: HashSet<(String, String)> = HashSet::new();
    let mut attrs = Vec::with_capacity(raw_attrs.len());
    for (aqname, value) in raw_attrs {
        let (aprefix, alocal) = split_qname(&aqname);
        let ans = if aprefix.is_empty() {
            String::new()
        } else {
            scope.get(aprefix).cloned().ok_or_else(|| {
                SamlError::InvalidMetadata(format!(
                    "unbound attribute namespace prefix '{aprefix}'"
                ))
            })?
        };
        if !seen_attr_keys.insert((ans.clone(), alocal.to_string())) {
            return Err(SamlError::SignatureWrapping(format!(
                "duplicate attribute '{alocal}' on '<{qname}>'"
            )));
        }
        if alocal == "ID" && ans.is_empty() && !value.is_empty() && !seen_ids.insert(value.clone())
        {
            return Err(SamlError::SignatureWrapping(format!(
                "duplicate XML ID attribute '{value}'"
            )));
        }
        attrs.push(XmlAttr {
            namespace_uri: ans,
            prefix: aprefix.to_string(),
            local_name: alocal.to_string(),
            qname: aqname,
            value,
        });
    }

    Ok((
        XmlElement {
            qname,
            prefix,
            local_name,
            namespace_uri,
            ns_decls,
            attrs,
            children: Vec::new(),
        },
        scope,
    ))
}

/// Public verification key parsed from an IdP X.509 certificate.
#[derive(Debug, Clone)]
pub(crate) enum IdpVerifyingKey {
    Rsa(RsaPublicKey),
    EcdsaP256(p256::ecdsa::VerifyingKey),
}

/// Extracts all X.509 certificates from PEM (`-----BEGIN CERTIFICATE-----`) or raw base64 blocks.
pub(crate) fn parse_idp_certificates(pem_or_b64: &str) -> Result<Vec<IdpVerifyingKey>, SamlError> {
    let mut ders: Vec<Vec<u8>> = Vec::new();
    if pem_or_b64.contains("-----BEGIN CERTIFICATE-----") {
        let mut remaining = pem_or_b64;
        while let Some(start) = remaining.find("-----BEGIN CERTIFICATE-----") {
            let after = &remaining[start + "-----BEGIN CERTIFICATE-----".len()..];
            let end = after.find("-----END CERTIFICATE-----").ok_or_else(|| {
                SamlError::InvalidCertificate("unterminated PEM certificate block".to_string())
            })?;
            let b64: String = after[..end]
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            let der = STANDARD.decode(b64).map_err(|e| {
                SamlError::InvalidCertificate(format!("invalid base64 inside PEM cert: {e}"))
            })?;
            ders.push(der);
            remaining = &after[end + "-----END CERTIFICATE-----".len()..];
        }
    } else {
        let b64: String = pem_or_b64.chars().filter(|c| !c.is_whitespace()).collect();
        if !b64.is_empty() {
            let der = STANDARD.decode(b64).map_err(|e| {
                SamlError::InvalidCertificate(format!("invalid base64 certificate: {e}"))
            })?;
            ders.push(der);
        }
    }

    let mut keys = Vec::with_capacity(ders.len());
    for der_bytes in ders {
        let spki = extract_spki_from_x509_der(&der_bytes)?;
        if let Ok(rsa_pub) = RsaPublicKey::from_public_key_der(&spki) {
            keys.push(IdpVerifyingKey::Rsa(rsa_pub));
        } else if let Ok(ec_pub) = p256::ecdsa::VerifyingKey::from_public_key_der(&spki) {
            keys.push(IdpVerifyingKey::EcdsaP256(ec_pub));
        } else {
            return Err(SamlError::InvalidCertificate(
                "certificate must carry an RSA or ECDSA P-256 public key".to_string(),
            ));
        }
    }
    if keys.is_empty() {
        return Err(SamlError::InvalidCertificate(
            "no X.509 certificates configured for identity provider".to_string(),
        ));
    }
    Ok(keys)
}

/// Parses SubjectPublicKeyInfo DER bytes directly from an ASN.1 DER X.509 Certificate.
pub(crate) fn extract_spki_from_x509_der(cert_der: &[u8]) -> Result<Vec<u8>, SamlError> {
    let mut reader = SliceReader::new(cert_der)
        .map_err(|e| SamlError::InvalidCertificate(format!("invalid DER certificate: {e}")))?;
    let spki = reader
        .sequence(|cert| {
            let spki_bytes = cert.sequence(|tbs| {
                if tbs.peek_tag()
                    == Ok(Tag::ContextSpecific {
                        constructed: true,
                        number: TagNumber::N0,
                    })
                {
                    let _version = tbs.tlv_bytes()?;
                }
                let _serial = tbs.tlv_bytes()?;
                let _sig_alg = tbs.tlv_bytes()?;
                let _issuer = tbs.tlv_bytes()?;
                let _validity = tbs.tlv_bytes()?;
                let _subject = tbs.tlv_bytes()?;
                let spki_tlv = tbs.tlv_bytes()?;
                while !tbs.is_finished() {
                    let _ = tbs.tlv_bytes()?;
                }
                Ok(spki_tlv.to_vec())
            })?;
            let _outer_sig_alg = cert.tlv_bytes()?;
            let _outer_sig_val = cert.tlv_bytes()?;
            Ok(spki_bytes)
        })
        .map_err(|e| SamlError::InvalidCertificate(format!("malformed X.509 ASN.1: {e}")))?;
    if !reader.is_finished() {
        return Err(SamlError::InvalidCertificate(
            "trailing bytes after X.509 certificate".to_string(),
        ));
    }
    Ok(spki)
}

/// Verifies XML-DSig signatures on a parsed SAML 2.0 `<Response>` against pinned IdP certs.
///
/// Rejects unsigned responses, partial assertion coverage, duplicate/nested assertions,
/// and unknown transforms or algorithms. Returns references to the verified `<Response>`
/// and its sole direct-child `<Assertion>`.
pub(crate) fn verify_saml_response_signatures<'a>(
    root: &'a XmlElement,
    pinned_keys: &[IdpVerifyingKey],
) -> Result<(&'a XmlElement, &'a XmlElement), SamlError> {
    if root.namespace_uri != XMLNS_SAML_PROTOCOL || root.local_name != "Response" {
        return Err(SamlError::InvalidMetadata(
            "SAML payload root must be samlp:Response".to_string(),
        ));
    }

    // Strict structural anti-XSW invariant:
    // Count every Assertion and EncryptedAssertion anywhere in the tree.
    let mut total_assertions = 0usize;
    let mut has_encrypted = false;
    count_assertions_recursive(root, &mut total_assertions, &mut has_encrypted);
    if has_encrypted {
        return Err(SamlError::InvalidMetadata(
            "EncryptedAssertion is not supported".to_string(),
        ));
    }
    if total_assertions != 1 {
        return Err(SamlError::SignatureWrapping(format!(
            "SAML Response must contain exactly 1 Assertion anywhere in the document (found {total_assertions})"
        )));
    }

    // The single Assertion MUST be a direct child of root <Response>.
    let assertion = root
        .find_direct_child(XMLNS_SAML_ASSERTION, "Assertion")
        .ok_or_else(|| {
            SamlError::SignatureWrapping(
                "Assertion must be a direct child of samlp:Response".to_string(),
            )
        })?;

    let resp_sig = root.find_direct_child(XMLNS_DSIG, "Signature");
    let assertion_sig = assertion.find_direct_child(XMLNS_DSIG, "Signature");

    if resp_sig.is_none() && assertion_sig.is_none() {
        return Err(SamlError::UnsignedResponse);
    }

    let mut root_ctx = BTreeMap::new();
    root_ctx.insert("xml".to_string(), XMLNS_XML.to_string());

    if let Some(sig) = resp_sig {
        verify_enveloped_signature(root, &root_ctx, root, sig, pinned_keys)?;
    }
    if let Some(sig) = assertion_sig {
        let mut assertion_parent_ctx = root_ctx;
        for (p, u) in &root.ns_decls {
            assertion_parent_ctx.insert(p.clone(), u.clone());
        }
        verify_enveloped_signature(
            assertion,
            &assertion_parent_ctx,
            assertion,
            sig,
            pinned_keys,
        )?;
    }

    Ok((root, assertion))
}

fn count_assertions_recursive(node: &XmlElement, count: &mut usize, has_encrypted: &mut bool) {
    if node.local_name == "Assertion" {
        *count += 1;
    } else if node.local_name == "EncryptedAssertion" {
        *has_encrypted = true;
    }
    for child in node.child_elements() {
        count_assertions_recursive(child, count, has_encrypted);
    }
}

fn verify_enveloped_signature(
    target: &XmlElement,
    target_parent_ns: &BTreeMap<String, String>,
    _signed_holder: &XmlElement,
    signature_node: &XmlElement,
    pinned_keys: &[IdpVerifyingKey],
) -> Result<(), SamlError> {
    let target_id = target.attr("ID").ok_or_else(|| {
        SamlError::SignatureVerification(format!(
            "signed element '{}' is missing ID attribute",
            target.local_name
        ))
    })?;

    let signed_info = signature_node
        .find_direct_child(XMLNS_DSIG, "SignedInfo")
        .ok_or_else(|| {
            SamlError::SignatureVerification("missing ds:SignedInfo element".to_string())
        })?;

    let sig_val_node = signature_node
        .find_direct_child(XMLNS_DSIG, "SignatureValue")
        .ok_or_else(|| {
            SamlError::SignatureVerification("missing ds:SignatureValue element".to_string())
        })?;

    let c14n_method = signed_info
        .find_direct_child(XMLNS_DSIG, "CanonicalizationMethod")
        .ok_or_else(|| {
            SamlError::SignatureVerification("missing ds:CanonicalizationMethod".to_string())
        })?;
    let c14n_alg = c14n_method.attr("Algorithm").unwrap_or("");
    if c14n_alg != ALG_EXC_C14N && c14n_alg != ALG_EXC_C14N_COMMENTS {
        return Err(SamlError::SignatureVerification(format!(
            "unsupported CanonicalizationMethod '{c14n_alg}'"
        )));
    }
    let signed_info_prefixes = parse_inclusive_prefixes(c14n_method);

    let sig_method = signed_info
        .find_direct_child(XMLNS_DSIG, "SignatureMethod")
        .and_then(|e| e.attr("Algorithm"))
        .unwrap_or("");
    if !matches!(
        sig_method,
        ALG_SIG_RSA_SHA256 | ALG_SIG_RSA_SHA512 | ALG_SIG_ECDSA_SHA256
    ) {
        return Err(SamlError::SignatureVerification(format!(
            "unsupported SignatureMethod '{sig_method}'"
        )));
    }

    let references: Vec<&XmlElement> = signed_info
        .child_elements()
        .filter(|e| e.namespace_uri == XMLNS_DSIG && e.local_name == "Reference")
        .collect();
    if references.len() != 1 {
        return Err(SamlError::SignatureWrapping(format!(
            "ds:SignedInfo must contain exactly 1 ds:Reference (found {})",
            references.len()
        )));
    }
    let reference = references[0];
    let expected_uri = format!("#{target_id}");
    if reference.attr("URI") != Some(expected_uri.as_str()) {
        return Err(SamlError::SignatureWrapping(format!(
            "ds:Reference URI mismatch (expected '{expected_uri}', found {:?})",
            reference.attr("URI")
        )));
    }

    // Validate Transforms: must include enveloped-signature and optionally exc-c14n.
    let mut ref_prefixes = BTreeSet::new();
    let mut saw_enveloped = false;
    if let Some(transforms) = reference.find_direct_child(XMLNS_DSIG, "Transforms") {
        for tr in transforms.child_elements() {
            if tr.namespace_uri != XMLNS_DSIG || tr.local_name != "Transform" {
                return Err(SamlError::SignatureWrapping(
                    "unexpected element inside ds:Transforms".to_string(),
                ));
            }
            let alg = tr.attr("Algorithm").unwrap_or("");
            if alg == ALG_ENVELOPED {
                saw_enveloped = true;
            } else if alg == ALG_EXC_C14N || alg == ALG_EXC_C14N_COMMENTS {
                ref_prefixes.extend(parse_inclusive_prefixes(tr));
            } else {
                return Err(SamlError::SignatureVerification(format!(
                    "unsupported XML-DSig transform '{alg}'"
                )));
            }
        }
    }
    if !saw_enveloped {
        return Err(SamlError::SignatureWrapping(
            "enveloped-signature transform is required".to_string(),
        ));
    }

    let digest_alg = reference
        .find_direct_child(XMLNS_DSIG, "DigestMethod")
        .and_then(|e| e.attr("Algorithm"))
        .unwrap_or("");
    let digest_b64: String = reference
        .find_direct_child(XMLNS_DSIG, "DigestValue")
        .ok_or_else(|| SamlError::SignatureVerification("missing ds:DigestValue".to_string()))?
        .text_content()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let expected_digest = STANDARD.decode(&digest_b64).map_err(|e| {
        SamlError::SignatureVerification(format!("invalid base64 DigestValue: {e}"))
    })?;

    // Compute canonicalized target element excluding this <ds:Signature>.
    let target_c14n = exclusive_c14n(
        target,
        target_parent_ns,
        &ref_prefixes,
        Some(signature_node as *const XmlElement),
    );
    let actual_digest: Vec<u8> = match digest_alg {
        ALG_DIGEST_SHA256 => Sha256::digest(target_c14n.as_bytes()).to_vec(),
        ALG_DIGEST_SHA512 => Sha512::digest(target_c14n.as_bytes()).to_vec(),
        other => {
            return Err(SamlError::SignatureVerification(format!(
                "unsupported DigestMethod '{other}'"
            )));
        }
    };

    if actual_digest.len() != expected_digest.len()
        || !bool::from(actual_digest.as_slice().ct_eq(expected_digest.as_slice()))
    {
        return Err(SamlError::SignatureVerification(
            "XML digest mismatch (response or assertion was modified)".to_string(),
        ));
    }

    // Compute ancestor namespaces for ds:SignedInfo (target_parent_ns + target + Signature).
    let mut signed_info_parent_ns = target_parent_ns.clone();
    for (p, u) in &target.ns_decls {
        signed_info_parent_ns.insert(p.clone(), u.clone());
    }
    for (p, u) in &signature_node.ns_decls {
        signed_info_parent_ns.insert(p.clone(), u.clone());
    }

    let signed_info_c14n = exclusive_c14n(
        signed_info,
        &signed_info_parent_ns,
        &signed_info_prefixes,
        None,
    );

    let sig_b64: String = sig_val_node
        .text_content()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let sig_bytes = STANDARD.decode(&sig_b64).map_err(|e| {
        SamlError::SignatureVerification(format!("invalid base64 SignatureValue: {e}"))
    })?;

    let verified = pinned_keys.iter().any(|key| match (sig_method, key) {
        (ALG_SIG_RSA_SHA256, IdpVerifyingKey::Rsa(rsa_pub)) => {
            if let Ok(sig) = RsaSignature::try_from(sig_bytes.as_slice()) {
                let vk = RsaVerifyingKey::<Sha256>::new(rsa_pub.clone());
                vk.verify(signed_info_c14n.as_bytes(), &sig).is_ok()
            } else {
                false
            }
        }
        (ALG_SIG_RSA_SHA512, IdpVerifyingKey::Rsa(rsa_pub)) => {
            if let Ok(sig) = RsaSignature::try_from(sig_bytes.as_slice()) {
                let vk = RsaVerifyingKey::<Sha512>::new(rsa_pub.clone());
                vk.verify(signed_info_c14n.as_bytes(), &sig).is_ok()
            } else {
                false
            }
        }
        (ALG_SIG_ECDSA_SHA256, IdpVerifyingKey::EcdsaP256(ec_pub)) => {
            if let Ok(sig) = p256::ecdsa::Signature::from_slice(sig_bytes.as_slice()) {
                ec_pub.verify(signed_info_c14n.as_bytes(), &sig).is_ok()
            } else {
                false
            }
        }
        _ => false,
    });

    if !verified {
        return Err(SamlError::SignatureVerification(
            "XML signature did not verify against any configured IdP certificate".to_string(),
        ));
    }

    Ok(())
}

fn parse_inclusive_prefixes(node: &XmlElement) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for child in node.child_elements() {
        if child.namespace_uri == XMLNS_EXC_C14N && child.local_name == "InclusiveNamespaces" {
            if let Some(list) = child.attr("PrefixList") {
                for token in list.split_whitespace() {
                    if token == "#default" {
                        set.insert(String::new());
                    } else {
                        set.insert(token.to_string());
                    }
                }
            }
        }
    }
    set
}

/// Performs W3C Exclusive Canonical XML 1.0 (`http://www.w3.org/2001/10/xml-exc-c14n#`).
pub(crate) fn exclusive_c14n(
    node: &XmlElement,
    parent_in_scope: &BTreeMap<String, String>,
    inclusive_prefixes: &BTreeSet<String>,
    skip_element_ptr: Option<*const XmlElement>,
) -> String {
    let mut out = String::new();
    let mut rendered_ns = BTreeMap::new();
    render_c14n_node(
        node,
        parent_in_scope,
        &mut rendered_ns,
        inclusive_prefixes,
        skip_element_ptr,
        &mut out,
    );
    out
}

fn render_c14n_node(
    node: &XmlElement,
    parent_in_scope: &BTreeMap<String, String>,
    rendered_ns: &mut BTreeMap<String, String>,
    inclusive_prefixes: &BTreeSet<String>,
    skip_element_ptr: Option<*const XmlElement>,
    out: &mut String,
) {
    if skip_element_ptr == Some(node as *const XmlElement) {
        return;
    }

    let mut in_scope = parent_in_scope.clone();
    for (p, u) in &node.ns_decls {
        in_scope.insert(p.clone(), u.clone());
    }

    // Collect visibly utilized namespace prefixes on `node`.
    let mut utilized = BTreeSet::new();
    utilized.insert(node.prefix.clone());
    for attr in &node.attrs {
        if !attr.prefix.is_empty() && attr.prefix != "xml" {
            utilized.insert(attr.prefix.clone());
        }
    }
    for inc in inclusive_prefixes {
        if in_scope.contains_key(inc) {
            utilized.insert(inc.clone());
        }
    }

    let saved_rendered = rendered_ns.clone();
    let mut ns_output: Vec<(String, String)> = Vec::new();

    for p in utilized {
        if p == "xml" {
            continue;
        }
        let current_uri = in_scope.get(&p).map(String::as_str).unwrap_or("");
        if p.is_empty() {
            let prev = rendered_ns.get("").map(String::as_str).unwrap_or("");
            if current_uri != prev {
                ns_output.push((String::new(), current_uri.to_string()));
                rendered_ns.insert(String::new(), current_uri.to_string());
            }
        } else if !current_uri.is_empty()
            && rendered_ns.get(&p).map(String::as_str) != Some(current_uri)
        {
            ns_output.push((p.clone(), current_uri.to_string()));
            rendered_ns.insert(p, current_uri.to_string());
        }
    }

    // Sort namespace nodes: default `xmlns` ("") precedes named prefixes sorted lexicographically.
    ns_output.sort_by(|a, b| a.0.cmp(&b.0));

    // Sort attributes lexicographically by (namespace_uri, local_name).
    let mut sorted_attrs: Vec<&XmlAttr> = node.attrs.iter().collect();
    sorted_attrs.sort_by(|a, b| {
        a.namespace_uri
            .cmp(&b.namespace_uri)
            .then_with(|| a.local_name.cmp(&b.local_name))
    });

    out.push('<');
    out.push_str(&node.qname);
    for (p, uri) in ns_output {
        if p.is_empty() {
            out.push_str(" xmlns=\"");
        } else {
            out.push_str(" xmlns:");
            out.push_str(&p);
            out.push_str("=\"");
        }
        push_c14n_attr_value(out, &uri);
        out.push('"');
    }
    for attr in sorted_attrs {
        out.push(' ');
        out.push_str(&attr.qname);
        out.push_str("=\"");
        push_c14n_attr_value(out, &attr.value);
        out.push('"');
    }
    out.push('>');

    for child in &node.children {
        match child {
            XmlChild::Text(t) => push_c14n_text(out, t),
            XmlChild::Element(elem) => render_c14n_node(
                elem,
                &in_scope,
                rendered_ns,
                inclusive_prefixes,
                skip_element_ptr,
                out,
            ),
        }
    }

    out.push_str("</");
    out.push_str(&node.qname);
    out.push('>');

    *rendered_ns = saved_rendered;
}

fn push_c14n_text(out: &mut String, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(ch),
        }
    }
}

fn push_c14n_attr_value(out: &mut String, val: &str) {
    for ch in val.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push_str("&#x9;"),
            '\n' => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            _ => out.push(ch),
        }
    }
}
