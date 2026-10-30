# QMS References (ISO 13485 pointers)

> **Scaffolding only.** This page lists where the software in this repository
> would hook into an organisation's quality management system. It does not
> create that system. An ISO 13485 certificate needs an actual QMS.

Each row names a clause, the artifact in this repo that supports it, and the
org-level process it depends on.

| ISO 13485 area | Clause (approx.) | Repo artifact | Org-level process it needs |
|---|---|---|---|
| Design and development controls | §7.3 | [software-development-plan.md](software-development-plan.md), `docs/ARCHITECTURE.md` | Design review, sign-off, design transfer |
| Design verification and validation | §7.3.5–7.3.7 | Test suite, `tpt-t-medical-sim` reports | Independent verification, validation on intended users |
| Document control | §4.2.4 | Git history, tagged releases, `CHANGELOG.md` | Controlled document register, approval workflow |
| Record control | §4.2.5 | Audit WAL (`tpt-t-medical-compliance`) | Retention schedule, archive, legibility checks |
| Risk management | §7.1 | [risk-management-file.md](risk-management-file.md) | ISO 14971 process owner |
| Purchasing and supplier control | §7.4 | [deny.toml](../../deny.toml) for dependencies | Supplier qualification, dependency audit |
| Corrective and preventive action (CAPA) | §8.5.2–8.5.3 | Issue tracker, `SECURITY.md` for vulnerabilities | CAPA board, root-cause process, effectiveness checks |
| Internal audit | §8.2.4 | None yet | Audit schedule, auditor independence |
| Complaint handling | §8.2.2 | `SECURITY.md` (security reports only) | Complaint intake, adverse-event reporting |
| Post-market surveillance | §8.2.1 | None yet | Field-data collection, trend analysis |
| Training and competence | §6.2 | None | Training records per role |
| Software validation for production | §4.1.6 | Build and test in CI (`.github/workflows/`) | Validated-tool register for the build chain |

## Gaps to close before certification

- Named process owners for each row above.
- Retention rules for audit logs and design records.
- A validated-tool register for `rustc`, `cargo`, and CI.
- A complaint and adverse-event workflow.
