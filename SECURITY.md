# Security policy

## Supported versions

Security fixes are made on the latest release line. Version 0.1.x is supported while it is the current release line.

## Reporting a vulnerability

Do not include exploit details, private feed data, tokens, or merchant information in a public issue. Use the repository's private vulnerability-reporting feature when available. If it is unavailable, contact the repository owner privately and request a secure reporting channel.

Include the affected version, operating system, minimal reproduction, expected impact, and whether the issue can expose or overwrite local data. You should receive an acknowledgment within seven days; timing for a fix depends on severity and reproducibility.

## Security boundary

Greenlit Goods is an offline parser and file-writing CLI. It never needs Google credentials. Treat feeds as untrusted input, review output paths, and do not run untrusted release binaries. The fixer refuses unsupported/lossy rewrites and never replaces an existing output file.
