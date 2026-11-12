use super::*;
use base64::engine::general_purpose::STANDARD;
use djangors_auth::SESSION_USER_ID_KEY;
use djangors_cache::InMemoryCache;
use djangors_core::AppState;
use djangors_db::config::DatabaseConfig;
use flate2::read::DeflateDecoder;
use hyper::http::{Extensions, HeaderMap, Method};
use rsa::pkcs1v15::SigningKey as RsaSigningKey;
use rsa::pkcs8::EncodePublicKey as _;
use rsa::signature::{SignatureEncoding as _, Signer as _};
use rsa::RsaPrivateKey;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read as _;

async fn setup_saml_db() -> Database {
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    let d = db.dialect();
    let pk = d.auto_pk_type();
    let ts = d.timestamp_type();
    let b = d.bool_type();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_user (
                    id {pk},
                    username VARCHAR(150) NOT NULL UNIQUE,
                    email VARCHAR(254) NOT NULL,
                    password TEXT NOT NULL,
                    is_active {b} NOT NULL,
                    is_staff {b} NOT NULL,
                    is_superuser {b} NOT NULL,
                    date_joined {ts} NOT NULL,
                    last_login {ts}
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    create_saml_tables(&db).await.unwrap();
    db
}

/// Builds a minimal DER-encoded ASN.1 TLV wrapper (`tag` + length + `content`).
fn der_tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(content.len() + 6);
    out.push(tag);
    if content.len() < 128 {
        out.push(content.len() as u8);
    } else {
        let len_bytes = (content.len() as u64).to_be_bytes();
        let trimmed: Vec<u8> = len_bytes.into_iter().skip_while(|b| *b == 0).collect();
        out.push(0x80 | (trimmed.len() as u8));
        out.extend_from_slice(&trimmed);
    }
    out.extend_from_slice(content);
    out
}

/// Wraps an `RsaPublicKey` SPKI inside a minimal syntactically valid X.509 v3 PEM certificate.
fn rsa_public_key_to_x509_pem(pub_key: &rsa::RsaPublicKey) -> String {
    let spki_der = pub_key.to_public_key_der().unwrap();
    let version = der_tlv(0xA0, &der_tlv(0x02, &[2]));
    let serial = der_tlv(0x02, &[1]);
    let sig_alg = der_tlv(0x30, &[0x06, 0x03, 0x55, 0x04, 0x03]);
    let issuer = der_tlv(0x30, &[]);
    let validity = der_tlv(0x30, &[]);
    let subject = der_tlv(0x30, &[]);

    let mut tbs_body = Vec::new();
    tbs_body.extend_from_slice(&version);
    tbs_body.extend_from_slice(&serial);
    tbs_body.extend_from_slice(&sig_alg);
    tbs_body.extend_from_slice(&issuer);
    tbs_body.extend_from_slice(&validity);
    tbs_body.extend_from_slice(&subject);
    tbs_body.extend_from_slice(spki_der.as_bytes());
    let tbs = der_tlv(0x30, &tbs_body);

    let outer_sig = der_tlv(0x03, &[0x00, 0x00]);
    let mut cert_body = Vec::new();
    cert_body.extend_from_slice(&tbs);
    cert_body.extend_from_slice(&sig_alg);
    cert_body.extend_from_slice(&outer_sig);
    let cert_der = der_tlv(0x30, &cert_body);
    let b64 = STANDARD.encode(&cert_der);
    format!("-----BEGIN CERTIFICATE-----\n{b64}\n-----END CERTIFICATE-----")
}

struct SignedSamlOptions<'a> {
    response_id: &'a str,
    assertion_id: &'a str,
    in_response_to: &'a str,
    recipient: &'a str,
    audience: &'a str,
    not_on_or_after: &'a str,
    name_id: &'a str,
    username: &'a str,
    email: &'a str,
}

fn build_signed_saml_response(private_key: &RsaPrivateKey, opts: &SignedSamlOptions<'_>) -> String {
    let unsigned_assertion = format!(
        "<saml:Assertion xmlns:saml=\"{XMLNS_SAML_ASSERTION}\" ID=\"{}\" Version=\"2.0\" IssueInstant=\"2026-01-01T00:00:00Z\">\
           <saml:Issuer>https://idp.example.com</saml:Issuer>\
           <saml:Subject>\
             <saml:NameID>{}</saml:NameID>\
             <saml:SubjectConfirmation Method=\"urn:oasis:names:tc:SAML:2.0:cm:bearer\">\
               <saml:SubjectConfirmationData NotOnOrAfter=\"{}\" Recipient=\"{}\" InResponseTo=\"{}\"/>\
             </saml:SubjectConfirmation>\
           </saml:Subject>\
           <saml:Conditions NotBefore=\"2020-01-01T00:00:00Z\" NotOnOrAfter=\"{}\">\
             <saml:AudienceRestriction>\
               <saml:Audience>{}</saml:Audience>\
             </saml:AudienceRestriction>\
           </saml:Conditions>\
           <saml:AttributeStatement>\
             <saml:Attribute Name=\"uid\"><saml:AttributeValue>{}</saml:AttributeValue></saml:Attribute>\
             <saml:Attribute Name=\"mail\"><saml:AttributeValue>{}</saml:AttributeValue></saml:Attribute>\
           </saml:AttributeStatement>\
         </saml:Assertion>",
        opts.assertion_id,
        opts.name_id,
        opts.not_on_or_after,
        opts.recipient,
        opts.in_response_to,
        opts.not_on_or_after,
        opts.audience,
        opts.username,
        opts.email,
    );

    let mut assertion_parent_ns = BTreeMap::new();
    assertion_parent_ns.insert("xml".to_string(), crate::xmldsig::XMLNS_XML.to_string());
    assertion_parent_ns.insert("samlp".to_string(), XMLNS_SAML_PROTOCOL.to_string());

    let parsed_assertion = parse_xml(&unsigned_assertion).unwrap();
    let assertion_c14n = crate::xmldsig::exclusive_c14n(
        &parsed_assertion,
        &assertion_parent_ns,
        &BTreeSet::new(),
        None,
    );
    let digest_b64 = STANDARD.encode(Sha256::digest(assertion_c14n.as_bytes()));

    let signed_info_xml = format!(
        "<ds:SignedInfo xmlns:ds=\"{XMLNS_DSIG}\">\
           <ds:CanonicalizationMethod Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"/>\
           <ds:SignatureMethod Algorithm=\"http://www.w3.org/2001/04/xmldsig-more#rsa-sha256\"/>\
           <ds:Reference URI=\"#{}\">\
             <ds:Transforms>\
               <ds:Transform Algorithm=\"http://www.w3.org/2000/09/xmldsig#enveloped-signature\"/>\
               <ds:Transform Algorithm=\"http://www.w3.org/2001/10/xml-exc-c14n#\"/>\
             </ds:Transforms>\
             <ds:DigestMethod Algorithm=\"http://www.w3.org/2001/04/xmlenc#sha256\"/>\
             <ds:DigestValue>{digest_b64}</ds:DigestValue>\
           </ds:Reference>\
         </ds:SignedInfo>",
        opts.assertion_id
    );

    let mut si_parent_ns = assertion_parent_ns;
    si_parent_ns.insert("saml".to_string(), XMLNS_SAML_ASSERTION.to_string());
    let parsed_si = parse_xml(&signed_info_xml).unwrap();
    let si_c14n = crate::xmldsig::exclusive_c14n(&parsed_si, &si_parent_ns, &BTreeSet::new(), None);

    let signer = RsaSigningKey::<Sha256>::new(private_key.clone());
    let sig_bytes = signer.sign(si_c14n.as_bytes()).to_vec();
    let sig_b64 = STANDARD.encode(sig_bytes);

    let signature_xml = format!(
        "<ds:Signature xmlns:ds=\"{XMLNS_DSIG}\">\
           {signed_info_xml}\
           <ds:SignatureValue>{sig_b64}</ds:SignatureValue>\
         </ds:Signature>"
    );

    let signed_assertion = unsigned_assertion.replacen(
        "<saml:Subject>",
        &format!("{signature_xml}<saml:Subject>"),
        1,
    );

    format!(
        "<samlp:Response xmlns:samlp=\"{XMLNS_SAML_PROTOCOL}\" ID=\"{}\" Version=\"2.0\" \
         IssueInstant=\"2026-01-01T00:00:00Z\" Destination=\"{}\" InResponseTo=\"{}\">\
           <saml:Issuer xmlns:saml=\"{XMLNS_SAML_ASSERTION}\">https://idp.example.com</saml:Issuer>\
           <samlp:Status><samlp:StatusCode Value=\"{SAML_STATUS_SUCCESS}\"/></samlp:Status>\
           {signed_assertion}\
         </samlp:Response>",
        opts.response_id, opts.recipient, opts.in_response_to
    )
}

#[tokio::test]
async fn idp_metadata_parsing_and_sp_metadata_generation() {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let cert_pem = rsa_public_key_to_x509_pem(&priv_key.to_public_key());
    let b64_cert = strip_pem_headers(&cert_pem);

    let idp_meta = format!(
        "<md:EntityDescriptor xmlns:md=\"urn:oasis:names:tc:SAML:2.0:metadata\" \
         xmlns:ds=\"http://www.w3.org/2000/09/xmldsig#\" entityID=\"https://idp.okta.com/exk123\">\
           <md:IDPSSODescriptor protocolSupportEnumeration=\"urn:oasis:names:tc:SAML:2.0:protocol\">\
             <md:KeyDescriptor use=\"signing\">\
               <ds:KeyInfo><ds:X509Data><ds:X509Certificate>{b64_cert}</ds:X509Certificate></ds:X509Data></ds:KeyInfo>\
             </md:KeyDescriptor>\
             <md:SingleSignOnService Binding=\"urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect\" \
               Location=\"https://idp.okta.com/app/sso/saml\"/>\
           </md:IDPSSODescriptor>\
         </md:EntityDescriptor>"
    );

    let db = setup_saml_db().await;
    let idp = SamlIdentityProvider::from_metadata_xml("okta", "Okta", idp_meta)
        .unwrap()
        .save(&db)
        .await
        .unwrap();
    assert_eq!(idp.entity_id, "https://idp.okta.com/exk123");
    assert_eq!(idp.sso_url, "https://idp.okta.com/app/sso/saml");
    assert_eq!(idp.verifying_certificates().unwrap().len(), 1);

    let state = AppState::new().insert(db).insert(SamlSpConfig::new(
        "https://app.example.com/saml/okta/metadata",
        "https://app.example.com",
    ));
    let mut params = PathParams::new();
    params.insert("idp", "okta");
    let req = Request::new(
        Method::GET,
        "/saml/okta/metadata".parse().unwrap(),
        HeaderMap::new(),
        hyper::body::Bytes::new(),
    )
    .with_state(state);
    let resp = saml_metadata_view(req, params).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let xml = std::str::from_utf8(resp.body()).unwrap();
    assert!(xml.contains("WantAssertionsSigned=\"true\""));
    assert!(xml.contains("https://app.example.com/saml/okta/acs"));
}

#[tokio::test]
async fn saml_login_acs_signature_verification_and_jit_user_creation() {
    let db = setup_saml_db().await;
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let cert_pem = rsa_public_key_to_x509_pem(&priv_key.to_public_key());

    let idp = SamlIdentityProvider {
        id: 0,
        slug: "okta".to_string(),
        name: "Okta SSO".to_string(),
        entity_id: "https://idp.example.com".to_string(),
        sso_url: "https://idp.example.com/sso".to_string(),
        x509_cert_pem: cert_pem,
        metadata_xml: String::new(),
        attribute_map_json: "{}".to_string(),
        jit_create_users: true,
        is_active: true,
        created_at: Utc::now(),
    }
    .save(&db)
    .await
    .unwrap();

    let cache: Arc<dyn Cache> = Arc::new(InMemoryCache::new(1000));
    let sp = SamlSpConfig::new("https://app.example.com/sp", "https://app.example.com");
    let state = AppState::new()
        .insert(db.clone())
        .insert(sp.clone())
        .insert(cache);

    let session = Session::new_empty();
    let mut ext = Extensions::new();
    ext.insert(session.clone());

    let mut params = PathParams::new();
    params.insert("idp", &idp.slug);

    // 1. Start SP-initiated login.
    let login_req = Request::new(
        Method::GET,
        "/saml/okta/login?next=/dashboard".parse().unwrap(),
        HeaderMap::new(),
        hyper::body::Bytes::new(),
    )
    .with_extensions(ext.clone())
    .with_state(state.clone());
    let login_resp = saml_login_view(login_req, params.clone()).await.unwrap();
    assert_eq!(login_resp.status(), StatusCode::FOUND);
    let loc = login_resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap();
    let parsed_url = Url::parse(loc).unwrap();
    let saml_req_b64 = parsed_url
        .query_pairs()
        .find(|(k, _)| k == "SAMLRequest")
        .unwrap()
        .1
        .into_owned();
    let relay_state = parsed_url
        .query_pairs()
        .find(|(k, _)| k == "RelayState")
        .unwrap()
        .1
        .into_owned();

    // Verify AuthnRequest inflates properly and inspect generated ID.
    let deflated = STANDARD.decode(&saml_req_b64).unwrap();
    let mut decoder = DeflateDecoder::new(&deflated[..]);
    let mut authn_xml = String::new();
    decoder.read_to_string(&mut authn_xml).unwrap();
    let parsed_authn = parse_xml(&authn_xml).unwrap();
    let req_id = parsed_authn.attr("ID").unwrap().to_string();

    // 2. Build a signed SAMLResponse matching req_id and post to ACS.
    let future_ts = (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
    let resp_xml = build_signed_saml_response(
        &priv_key,
        &SignedSamlOptions {
            response_id: "_resp_1",
            assertion_id: "_assert_1",
            in_response_to: &req_id,
            recipient: "https://app.example.com/saml/okta/acs",
            audience: "https://app.example.com/sp",
            not_on_or_after: &future_ts,
            name_id: "alice@example.com",
            username: "alice",
            email: "alice@example.com",
        },
    );
    let resp_b64 = STANDARD.encode(resp_xml.as_bytes());
    let form_body = serde_urlencoded::to_string([
        ("SAMLResponse", resp_b64.as_str()),
        ("RelayState", relay_state.as_str()),
    ])
    .unwrap();
    let mut hdrs = HeaderMap::new();
    hdrs.insert(
        "content-type",
        "application/x-www-form-urlencoded".parse().unwrap(),
    );
    let acs_req = Request::new(
        Method::POST,
        "/saml/okta/acs".parse().unwrap(),
        hdrs,
        hyper::body::Bytes::from(form_body),
    )
    .with_extensions(ext)
    .with_state(state);

    let acs_resp = saml_acs_view(acs_req, params).await.unwrap();
    assert_eq!(acs_resp.status(), StatusCode::FOUND);
    assert_eq!(
        acs_resp
            .headers()
            .get("location")
            .unwrap()
            .to_str()
            .unwrap(),
        "/dashboard"
    );

    let uid = session.get::<i64>(SESSION_USER_ID_KEY).unwrap();
    let user = User::objects()
        .filter(djangors_orm::q!(id = uid))
        .unwrap()
        .get(&db)
        .await
        .unwrap();
    assert_eq!(user.username, "alice");
    assert_eq!(user.email, "alice@example.com");
    assert_eq!(user.password, "!");
}

#[tokio::test]
async fn rejects_unsigned_tampered_xsw_replay_expired_audience_and_recipient() {
    let mut rng = rand::thread_rng();
    let priv_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let rogue_key = RsaPrivateKey::new(&mut rng, 2048).unwrap();
    let cert_pem = rsa_public_key_to_x509_pem(&priv_key.to_public_key());

    let idp = SamlIdentityProvider {
        id: 1,
        slug: "okta".to_string(),
        name: "Okta".to_string(),
        entity_id: "https://idp.example.com".to_string(),
        sso_url: "https://idp.example.com/sso".to_string(),
        x509_cert_pem: cert_pem,
        metadata_xml: String::new(),
        attribute_map_json: "{}".to_string(),
        jit_create_users: true,
        is_active: true,
        created_at: Utc::now(),
    };
    let sp = SamlSpConfig::new("https://app.example.com/sp", "https://app.example.com");
    let cache = InMemoryCache::new(1000);
    let future_ts = (Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();

    let valid_xml = build_signed_saml_response(
        &priv_key,
        &SignedSamlOptions {
            response_id: "_resp_1",
            assertion_id: "_assert_1",
            in_response_to: "_req_1",
            recipient: "https://app.example.com/saml/okta/acs",
            audience: "https://app.example.com/sp",
            not_on_or_after: &future_ts,
            name_id: "alice@example.com",
            username: "alice",
            email: "alice@example.com",
        },
    );

    // 1. Unsigned assertion rejected.
    let sig_start = valid_xml.find("<ds:Signature").unwrap();
    let sig_end = valid_xml.find("</ds:Signature>").unwrap() + "</ds:Signature>".len();
    let unsigned_xml = format!("{}{}", &valid_xml[..sig_start], &valid_xml[sig_end..]);
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&unsigned_xml),
        &idp,
        &sp,
        Some("_req_1"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::UnsignedResponse));

    // 2. Tampering with signed assertion body fails digest verification.
    let tampered_xml = valid_xml.replacen(">alice<", ">admin<", 1);
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&tampered_xml),
        &idp,
        &sp,
        Some("_req_1"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::SignatureVerification(_)));

    // 3. Signature signed by rogue private key (not pinned in IdP record) fails.
    let rogue_xml = build_signed_saml_response(
        &rogue_key,
        &SignedSamlOptions {
            response_id: "_resp_r",
            assertion_id: "_assert_r",
            in_response_to: "_req_1",
            recipient: "https://app.example.com/saml/okta/acs",
            audience: "https://app.example.com/sp",
            not_on_or_after: &future_ts,
            name_id: "evil@example.com",
            username: "evil",
            email: "evil@example.com",
        },
    );
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&rogue_xml),
        &idp,
        &sp,
        Some("_req_1"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::SignatureVerification(_)));

    // 4. XSW: second assertion injected into Response is rejected.
    let xsw_extra = valid_xml.replacen(
        "</samlp:Response>",
        "<saml:Assertion xmlns:saml=\"urn:oasis:names:tc:SAML:2.0:assertion\" ID=\"_evil\"><saml:Subject><saml:NameID>admin</saml:NameID></saml:Subject></saml:Assertion></samlp:Response>",
        1,
    );
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&xsw_extra),
        &idp,
        &sp,
        Some("_req_1"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::SignatureWrapping(_)));

    // 5. XSW: duplicate ID attribute rejected.
    let dup_id = valid_xml.replacen("_resp_1", "_assert_1", 1);
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&dup_id),
        &idp,
        &sp,
        Some("_req_1"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::SignatureWrapping(_)));

    // 6. DOCTYPE / DTD rejected.
    let dtd_xml = format!("<!DOCTYPE foo>{valid_xml}");
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&dtd_xml),
        &idp,
        &sp,
        Some("_req_1"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::SignatureWrapping(_)));

    // 7. Wrong InResponseTo rejected before replay caching!
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&valid_xml),
        &idp,
        &sp,
        Some("_wrong_req"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::InResponseToMismatch(_)));

    // 8. First valid verification succeeds; immediate replay fails with ReplayDetected!
    let claims = verify_and_extract_saml_response(
        &STANDARD.encode(&valid_xml),
        &idp,
        &sp,
        Some("_req_1"),
        &cache,
    )
    .await
    .unwrap();
    assert_eq!(claims.assertion_id, "_assert_1");

    let replay_err = verify_and_extract_saml_response(
        &STANDARD.encode(&valid_xml),
        &idp,
        &sp,
        Some("_req_1"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(replay_err, SamlError::ReplayDetected(_)));

    // 9. Expired NotOnOrAfter rejected.
    let past_ts = (Utc::now() - chrono::Duration::minutes(10)).to_rfc3339();
    let expired_xml = build_signed_saml_response(
        &priv_key,
        &SignedSamlOptions {
            response_id: "_resp_exp",
            assertion_id: "_assert_exp",
            in_response_to: "_req_exp",
            recipient: "https://app.example.com/saml/okta/acs",
            audience: "https://app.example.com/sp",
            not_on_or_after: &past_ts,
            name_id: "alice@example.com",
            username: "alice",
            email: "alice@example.com",
        },
    );
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&expired_xml),
        &idp,
        &sp,
        Some("_req_exp"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::ExpiredAssertion(_)));

    // 10. Wrong Audience rejected.
    let wrong_aud_xml = build_signed_saml_response(
        &priv_key,
        &SignedSamlOptions {
            response_id: "_resp_aud",
            assertion_id: "_assert_aud",
            in_response_to: "_req_aud",
            recipient: "https://app.example.com/saml/okta/acs",
            audience: "https://other.example.com/sp",
            not_on_or_after: &future_ts,
            name_id: "alice@example.com",
            username: "alice",
            email: "alice@example.com",
        },
    );
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&wrong_aud_xml),
        &idp,
        &sp,
        Some("_req_aud"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::AudienceMismatch { .. }));

    // 11. Wrong Recipient rejected.
    let wrong_rec_xml = build_signed_saml_response(
        &priv_key,
        &SignedSamlOptions {
            response_id: "_resp_rec",
            assertion_id: "_assert_rec",
            in_response_to: "_req_rec",
            recipient: "https://evil.example.com/acs",
            audience: "https://app.example.com/sp",
            not_on_or_after: &future_ts,
            name_id: "alice@example.com",
            username: "alice",
            email: "alice@example.com",
        },
    );
    let err = verify_and_extract_saml_response(
        &STANDARD.encode(&wrong_rec_xml),
        &idp,
        &sp,
        Some("_req_rec"),
        &cache,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, SamlError::RecipientMismatch { .. }));
}
