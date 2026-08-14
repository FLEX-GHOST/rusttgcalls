# Security Policy

`rusttgcalls` takes security, memory safety, and data confidentiality seriously. This document outlines our supported versions, security architecture, and responsible vulnerability disclosure process.

---

## Supported Versions

| Version | Supported |
| :--- | :--- |
| `0.1.x` | Yes |
| `< 0.1.0` | No |

---

## Security Architecture

- **Pure Rust Invariants:** 100% safe Rust code with zero undefined behavior, data races, or memory corruption vulnerabilities.
- **End-to-End Encryption:** All WebRTC media traffic is protected with DTLS 1.2 and SRTP (AES-128-GCM / AES-CM-128-HMAC-SHA1-80) adhering strictly to standard cryptographic specifications.
- **Subprocess Isolation:** Media transcoding runs in bounded subprocess workers with restricted resource limits to isolate media parsing from bot runtime memory.
- **Blob-Only Signalling:** The library never stores or exposes MTProto credentials or session keys; all signalling is handled via opaque blob parameters.

---

## Reporting a Vulnerability

If you discover a security vulnerability in `rusttgcalls`, please report it responsibly:

1. **Do not open a public GitHub issue.**
2. Use GitHub's private security advisory feature under the **Security** tab of the repository (`Security -> Advisories -> Report a vulnerability`).
3. Or contact the maintainer directly on Telegram:
   <a href="https://t.me/a22bq"><img src="https://img.shields.io/badge/Telegram-@a22bq-0088cc.svg?logo=telegram&logoColor=white" alt="Security Contact"></a>
4. Provide detailed steps to reproduce the issue, including:
   - Affected version and operating system.
   - Minimal reproduction code or payload.
   - Potential impact of the vulnerability.

We will acknowledge receipt of your report within 48 hours, validate the findings, and coordinate a patch release before public disclosure.
