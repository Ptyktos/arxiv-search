# Security Policy

## Supported Versions

We currently support the latest version of the `arxiv-search-rs-mcp` on the `master` branch. As this is a continuously deployed library and server, we strongly recommend users pull the latest changes frequently.

| Version | Supported          |
| ------- | ------------------ |
| `master`| :white_check_mark: |
| Older   | :x:                |

## Reporting a Vulnerability

We take the security of `arxiv-search-rs-mcp` seriously. If you discover a security vulnerability within this project, please responsibly disclose the information.

Please **do not** open a public issue for security vulnerabilities. Instead, please report the vulnerability privately by opening a Draft Security Advisory on GitHub or by contacting the repository maintainers directly.

When reporting a vulnerability, please include:
* A detailed description of the vulnerability.
* Steps to reproduce the issue.
* Any potential impact or risk associated with the vulnerability.
* Suggested mitigations if you have them.

We will acknowledge your report as quickly as possible and work with you to understand and resolve the issue.

## Threat Model & Out of Scope

Note that the following are generally considered out of scope for security vulnerabilities:
- Upstream issues in the `export.arxiv.org` API or Semantic Scholar APIs.
- Volumetric Denial of Service (DoS) attacks targeted at the hosted Cloudflare Worker.
- Security vulnerabilities in the local environment running the MCP server, unless the server provides a vector to compromise the host system through maliciously crafted API payloads.
