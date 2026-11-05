# Security Policy

## Reporting Vulnerabilities

If you discover a security vulnerability in Djangors, please report it privately so we can address the issue before public disclosure.

- **Primary Channel**: Use GitHub's [Private Vulnerability Reporting](https://github.com/QUANTORA-ltd/djangors/security/advisories/new) (navigate to the **Security** tab on the repository and click **Report a vulnerability**).
- **Fallback Channel**: Email `chidiisking7@gmail.com`.

> [!IMPORTANT]
> **Do NOT open a public GitHub issue for a security bug.** Publicly disclosing vulnerabilities before a patch is available exposes users to potential exploits before a fix exists.

### Response Expectations

Djangors is maintained by a single developer. Response expectations reflect a single-maintainer open-source project:
- **Acknowledgement**: We aim to acknowledge receipt of security reports within 48 to 72 hours (business days).
- **Assessment & Fix**: We will assess the vulnerability, provide updates on progress, and release a fix as quickly as practical. We do not maintain a strict 24-hour SLA.

## Supported Versions

Djangors is currently in pre-1.0 active development (version `0.6.0`).

| Version | Supported |
| ------- | --------- |
| 0.6.x   | Yes       |
| < 0.6.0 | No        |

- Only the latest `0.x` release line receives security fixes and maintenance updates.
- There are no Long-Term Support (LTS) branches or backports for older releases.

## Scope

### In Scope
The following framework subsystems and features are within security scope:
- **Authentication & Sessions**: Session cookie signing, password hashing (Argon2id), active-user checks, and admin permission logic (`djangors-auth`, `djangors-sessions`).
- **CSRF Protection**: CSRF token generation, header validation, and form-body field (`csrfmiddlewaretoken`) validation middleware.
- **Admin Permissions**: Per-action and model permission enforcement (`require_perm`, `has_perm`).
- **ORM & Injection Surface**: SQL query generation and parameter binding (`djangors-orm`).
- **Static Files**: Local disk storage path-traversal and symlink escape defenses (`LocalDiskStorage`).
- **Request Extractors**: HTTP header, query parameter, JSON, and multipart form-body parsing.

### Out of Scope
The following items are explicitly **NOT** in security scope:
- **Example Applications**: Demo applications under `examples/polls` and `examples/school` are for reference and demonstration purposes.
- **Development Debug Mode (`DEBUG=true`)**: The development error debug page is intentionally verbose to assist debugging and is not a security vulnerability when explicitly enabled.

## Known & Triaged Accepted Risks

Before reporting a security advisory, please review our documented security review in [`docs/security-review-2026-07-27.md`](docs/security-review-2026-07-27.md) and the following three triaged advisories:

1. **RUSTSEC-2026-0194** (`quick-xml` 0.40.1, transitive via `s3` crate):
   - Denial-of-service via quadratic-runtime duplicate attribute check. Currently unfixable from this project because the `s3` crate pins `quick-xml = "0.40.1"`.
2. **RUSTSEC-2026-0195** (`quick-xml` 0.40.1, transitive via `s3` crate):
   - Unbounded namespace allocation enabling memory exhaustion DoS. Currently unfixable upstream until `s3` publishes a release with an updated `quick-xml` requirement.
3. **RUSTSEC-2023-0071** (`rsa` 0.9.10, transitive via `djangors-rest` optional `jwt` feature):
   - Marvin Attack timing side-channel in `rsa` (via `jsonwebtoken`). No upstream fix exists; projects enabling `jwt` with RSA algorithms should prefer HMAC (HS256) or ECDSA (ES256) if timing side-channels are a concern for their threat model.
