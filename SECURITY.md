# Security policy

## Reporting a vulnerability

Please report vulnerabilities privately through
[GitHub Security Advisories](https://github.com/oRIOnTx/power-idea/security/advisories/new),
not in public issues. Include the version (Welcome screen → click the version), steps to reproduce
and the impact you see.

You can expect an acknowledgement within 7 days and a fix or mitigation plan within 30 days for
confirmed issues. Credit is given in the release notes unless you prefer otherwise.

## Supported versions

Only the latest release receives security fixes.

## Scope

Of particular interest:

- the MCP server (it must bind to 127.0.0.1, require the bearer token, reject non-local origins and
  expose only read-only tools plus UI navigation);
- the extension host (paths from extensions must stay inside the project; extensions are never loaded
  from a project folder);
- handling of secrets: database passwords in the Keychain, the MCP token file, `.env` files.

Dependencies are checked by cargo-deny on every change and by a daily `cargo audit`.
