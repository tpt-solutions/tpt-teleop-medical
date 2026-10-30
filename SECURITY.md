# Security Policy

## Scope

This repository contains safety-critical and security-sensitive software for
surgical robotics and hospital automation. Reports are handled as potential
patient-safety issues first and as software vulnerabilities second.

Areas of particular concern:

- Ed25519 signing and key handling for the audit trail (`tpt-t-medical-compliance`)
- Hash-chain tamper evidence and log sealing
- PHI encryption at rest and in transit (`tpt-t-medical-hipaa`, AES-256-GCM)
- Biometric and badge authentication for narcotic chain-of-custody
- Command authority: operator override, virtual-fixture enforcement and
  emergency-stop paths (`tpt-t-medical-safety`)
- Parsers for untrusted network input (HL7v2, FHIR, DICOM)

## Reporting a vulnerability

Please do not open a public issue for security problems.

Report privately to **[SECURITY CONTACT: set before publishing]** with the
subject line `[tpt-teleop-medical security]`. Include:

- The affected crate, file or commit.
- Steps to reproduce, or a proof of concept.
- The impact you believe it has, including any effect on patient safety.

We aim to acknowledge reports within 3 business days and to agree a disclosure
timeline with the reporter. Reports that could affect patient safety are
triaged ahead of all other work.

## Supported versions

No release has been published yet. Until v1.0.0 is tagged, only the `master`
branch is supported.

## Out of scope

- Findings that require physical access to a deployed device without a
  documented threat model.
- Issues in upstream dependencies that have no effect on this project's use
  of them. Report those to the upstream project.
