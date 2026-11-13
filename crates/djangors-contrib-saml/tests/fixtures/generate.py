"""Generate SAML response fixtures signed by signxml (libxml2 exclusive c14n),
an implementation independent of djangors-contrib-saml's own c14n.

Usage: samlvenv/bin/python -I gen_saml_fixtures.py OUT_DIR
"""
import datetime
import sys
from pathlib import Path

from cryptography import x509
from cryptography.hazmat.primitives import hashes, serialization
from cryptography.hazmat.primitives.asymmetric import ec, rsa
from cryptography.x509.oid import NameOID
from lxml import etree
from signxml import XMLSigner, SignatureMethod, DigestAlgorithm, SignatureConstructionMethod

OUT = Path(sys.argv[1])
OUT.mkdir(parents=True, exist_ok=True)

SAMLP = "urn:oasis:names:tc:SAML:2.0:protocol"
SAML = "urn:oasis:names:tc:SAML:2.0:assertion"
DS = "http://www.w3.org/2000/09/xmldsig#"
C14N = "http://www.w3.org/2001/10/xml-exc-c14n#"

ACS = "https://app.example.com/saml/okta/acs"
AUDIENCE = "https://app.example.com/sp"
IDP = "https://idp.example.com"


def make_cert(key, cn):
    name = x509.Name([x509.NameAttribute(NameOID.COMMON_NAME, cn)])
    now = datetime.datetime(2025, 1, 1, tzinfo=datetime.timezone.utc)
    return (
        x509.CertificateBuilder()
        .subject_name(name)
        .issuer_name(name)
        .public_key(key.public_key())
        .serial_number(x509.random_serial_number())
        .not_valid_before(now)
        .not_valid_after(now + datetime.timedelta(days=36500))
        .sign(key, hashes.SHA256())
    )


def pem(cert):
    return cert.public_bytes(serialization.Encoding.PEM).decode()


rsa_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
rsa_cert = make_cert(rsa_key, "fixture-idp-rsa")
ec_key = ec.generate_private_key(ec.SECP256R1())
ec_cert = make_cert(ec_key, "fixture-idp-ec")
rogue_key = rsa.generate_private_key(public_exponent=65537, key_size=2048)
rogue_cert = make_cert(rogue_key, "fixture-rogue")

(OUT / "idp_rsa_cert.pem").write_text(pem(rsa_cert))
(OUT / "idp_ec_cert.pem").write_text(pem(ec_cert))


def assertion_xml(aid, name_id, irt="_req_fixture_1"):
    # Formatted with whitespace and newlines, the way real IdPs (Okta, Entra) emit it.
    return f"""<saml2:Assertion xmlns:saml2="{SAML}" xmlns:xs="http://www.w3.org/2001/XMLSchema" ID="{aid}" IssueInstant="2026-01-01T00:00:00Z" Version="2.0">
    <saml2:Issuer Format="urn:oasis:names:tc:SAML:2.0:nameid-format:entity">{IDP}</saml2:Issuer>
    <ds:Signature xmlns:ds="{DS}" Id="placeholder"></ds:Signature>
    <saml2:Subject>
      <saml2:NameID Format="urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress">{name_id}</saml2:NameID>
      <saml2:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer">
        <saml2:SubjectConfirmationData InResponseTo="{irt}" NotOnOrAfter="2099-01-01T00:00:00Z" Recipient="{ACS}"/>
      </saml2:SubjectConfirmation>
    </saml2:Subject>
    <saml2:Conditions NotBefore="2020-01-01T00:00:00Z" NotOnOrAfter="2099-01-01T00:00:00Z">
      <saml2:AudienceRestriction>
        <saml2:Audience>{AUDIENCE}</saml2:Audience>
      </saml2:AudienceRestriction>
    </saml2:Conditions>
    <saml2:AuthnStatement AuthnInstant="2026-01-01T00:00:00Z" SessionIndex="{aid}">
      <saml2:AuthnContext>
        <saml2:AuthnContextClassRef>urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport</saml2:AuthnContextClassRef>
      </saml2:AuthnContext>
    </saml2:AuthnStatement>
    <saml2:AttributeStatement>
      <saml2:Attribute Name="uid" NameFormat="urn:oasis:names:tc:SAML:2.0:attrname-format:basic">
        <saml2:AttributeValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="xs:string">fixture-alice</saml2:AttributeValue>
      </saml2:Attribute>
      <saml2:Attribute Name="mail" NameFormat="urn:oasis:names:tc:SAML:2.0:attrname-format:basic">
        <saml2:AttributeValue xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:type="xs:string">alice@example.com</saml2:AttributeValue>
      </saml2:Attribute>
    </saml2:AttributeStatement>
  </saml2:Assertion>"""


def response_xml(rid, assertion, irt="_req_fixture_1", with_placeholder=False):
    ph = f'\n  <ds:Signature xmlns:ds="{DS}" Id="placeholder"></ds:Signature>' if with_placeholder else ""
    return f"""<saml2p:Response xmlns:saml2p="{SAMLP}" Destination="{ACS}" ID="{rid}" InResponseTo="{irt}" IssueInstant="2026-01-01T00:00:00Z" Version="2.0">
  <saml2:Issuer xmlns:saml2="{SAML}" Format="urn:oasis:names:tc:SAML:2.0:nameid-format:entity">{IDP}</saml2:Issuer>{ph}
  <saml2p:Status>
    <saml2p:StatusCode Value="urn:oasis:names:tc:SAML:2.0:status:Success"/>
  </saml2p:Status>
  {assertion}
</saml2p:Response>"""


def strip_placeholder(xml):
    return xml.replace(f'\n    <ds:Signature xmlns:ds="{DS}" Id="placeholder"></ds:Signature>', "")


def sign(xml_str, key, cert, method, digest, ref_id):
    root = etree.fromstring(xml_str.encode())
    signer = XMLSigner(
        method=SignatureConstructionMethod.enveloped,
        signature_algorithm=method,
        digest_algorithm=digest,
        c14n_algorithm=C14N,
    )
    signer.namespaces = {"ds": DS}
    signed = signer.sign(root, key=key, cert=pem(cert), reference_uri=ref_id)
    return etree.tostring(signed).decode()


def signed_assertion(aid, name_id, key, cert, method=SignatureMethod.RSA_SHA256, digest=DigestAlgorithm.SHA256, irt="_req_fixture_1"):
    return sign(assertion_xml(aid, name_id, irt), key, cert, method, digest, aid)


def write(name, xml):
    (OUT / name).write_text(xml)


# 1. Assertion signed with RSA-SHA256, Response unsigned (Okta default).
write("rsa_sha256_signed_assertion.xml",
      response_xml("_resp_f1", signed_assertion("_assert_f1", "alice@example.com", rsa_key, rsa_cert)))

# 2. Assertion signed with RSA-SHA512 / SHA-512 digest.
write("rsa_sha512_signed_assertion.xml",
      response_xml("_resp_f2", signed_assertion("_assert_f2", "alice@example.com", rsa_key, rsa_cert,
                                                SignatureMethod.RSA_SHA512, DigestAlgorithm.SHA512)))

# 3. Assertion signed with ECDSA P-256 / SHA-256.
write("ecdsa_p256_signed_assertion.xml",
      response_xml("_resp_f3", signed_assertion("_assert_f3", "alice@example.com", ec_key, ec_cert,
                                                SignatureMethod.ECDSA_SHA256, DigestAlgorithm.SHA256)))

# 4. Only the Response is signed (assertion unsigned) — covers the assertion as a descendant.
unsigned_assert = strip_placeholder(assertion_xml("_assert_f4", "alice@example.com"))
write("rsa_signed_response_only.xml",
      sign(response_xml("_resp_f4", unsigned_assert, with_placeholder=True), rsa_key, rsa_cert,
           SignatureMethod.RSA_SHA256, DigestAlgorithm.SHA256, "_resp_f4"))

# 5. Both Response and Assertion signed (ADFS / Entra "sign response and assertion").
write("rsa_signed_response_and_assertion.xml",
      sign(response_xml("_resp_f5", signed_assertion("_assert_f5", "alice@example.com", rsa_key, rsa_cert),
                        with_placeholder=True),
           rsa_key, rsa_cert, SignatureMethod.RSA_SHA256, DigestAlgorithm.SHA256, "_resp_f5"))

# 6. Signed by a key that is NOT the pinned IdP cert (embeds the rogue cert in KeyInfo).
write("rogue_key_signed_assertion.xml",
      response_xml("_resp_f6", signed_assertion("_assert_f6", "alice@example.com", rogue_key, rogue_cert)))

# 7. Comment-injection: NameID text split by a comment. c14n drops comments, so the
#    signature is valid; a correct SP must see the full value "admin@example.com.evil.test".
ci = signed_assertion("_assert_f7", "admin@example.com<!---->.evil.test", rsa_key, rsa_cert)
assert "<!---->" in ci, "comment must survive serialization"
write("comment_injection_nameid.xml", response_xml("_resp_f7", ci))

print("wrote fixtures to", OUT)
