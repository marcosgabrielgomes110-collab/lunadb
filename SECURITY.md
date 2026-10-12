# Security Policy

## Supported versions

LunaDB is at an early stage. Security fixes are applied to the latest `0.0.x` release
on the `main` branch.

| Version | Supported |
|---------|-----------|
| 0.0.x   | ✅        |

## Reporting a vulnerability

Please **do not** open a public issue for security problems. Instead, report privately
using GitHub's [security advisories](https://github.com/marcosgabrielgomes110-collab/lunadb/security/advisories/new)
or by email to marcosgabrielgomes110@gmail.com.

Include as much of the following as possible:

- A description of the issue and its impact.
- Steps to reproduce (or a proof of concept).
- Affected version/commit and configuration.
- Any suggested fix or mitigation.

You will receive an acknowledgement as soon as possible, and we will work with you on
a coordinated disclosure.

## Security notes

- Change the default `root` password (`changeme`) before exposing the server publicly.
- The HTTP API does not terminate TLS. Run it behind a TLS reverse proxy on untrusted networks.
- Keep the master key (`~/.lunadb/meta/master.key`, mode `0600`) secret; losing it makes
  encrypted fields unreadable.
- API keys are shown only once, at creation/login time. Store them securely and revoke unused keys.
