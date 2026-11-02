# Security Policy

## Supported versions

Security fixes go into the latest release. Until 1.0, only the most recent
minor version is supported.

## Reporting a vulnerability

Please **do not** open a public issue. Report it privately through
[GitHub Security Advisories](https://github.com/meetdhruva/difflux/security/advisories/new).

Include what you found, how to reproduce it, and the impact you expect.
You should get a reply within 7 days. Once a fix is ready we publish a
release and an advisory, and credit you unless you prefer otherwise.

## Scope

difflux reads the files and folders you point it at. Issues that matter most:

- Anything that makes difflux modify, delete or corrupt user files.
- Crashes or hangs triggered by crafted input files or folder trees.
- The MCP server (`difflux mcp`) reading or exposing data beyond the paths requested.

## Verifying downloads

Every release asset has a SHA-256 checksum in `SHA256SUMS` and a signed
build provenance attestation. To verify:

```sh
sha256sum --check --ignore-missing SHA256SUMS
gh attestation verify difflux-<version>-<platform>.tar.gz --repo meetdhruva/difflux
```
