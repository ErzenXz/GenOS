# Security policy

GenOS is an experimental operating system and is not yet appropriate for sensitive data or production workloads. Security reports are still valuable because early findings can shape safer boundaries before they become compatibility commitments.

## Reporting a vulnerability

Please do **not** open a public issue for a suspected vulnerability.

Use GitHub's private vulnerability reporting flow:

1. Open the repository's **Security** tab.
2. Choose **Report a vulnerability**.
3. Include the affected commit, subsystem, reproduction steps, impact, and any suggested mitigation.

Reports about memory safety, privilege boundaries, boot trust, filesystem corruption, malformed hardware input, or build/release integrity are especially useful.

You should receive an initial acknowledgment within seven days. Because GenOS is currently maintained as an experimental project, remediation timelines depend on impact and maintainer availability. Coordinated disclosure details will be agreed with the reporter before publication.

## Supported versions

Only the current `main` branch is supported during the experimental stage. There are no stable security-maintenance releases yet.

## Current security limitations

GenOS does not yet provide a complete production security model. In particular, the current roadmap still includes userspace isolation, identities, capabilities, persistent-storage protection, cryptographic entropy, and signed updates. See [ROADMAP.md](ROADMAP.md) for planned security milestones.
