# Risk Management File (ISO 14971 template)

> **Scaffolding only.** This template is a starting point for a certification
> effort. It is not a certified risk file. Hazard analysis must be carried out
> by a qualified, multi-disciplinary team, and the results must be reviewed by
> the organisation's risk-management process.

Document ID: `RMF-TPT-MED-001`
Version: `0.1-draft`

## 1. Risk management plan

- Risk acceptability criteria: `[define: e.g. risk matrix, ALARP]`.
- Hazard sources in scope: software, hardware, network, human factors, and
  use environment.
- Seed sources: spec.txt §4 (safety mechanisms) and §5.5 (safety and
  redundancy).

## 2. Hazard identification

| ID | Hazard | Hazardous situation | Harm | Seed source |
|---|---|---|---|---|
| H-001 | Unintended tool motion outside workspace envelope | Tool leaves the safe volume | Tissue damage | spec §5.5 |
| H-002 | Force/torque spike on unexpected resistance | Tool meets bone or a vessel | Tissue damage, bleeding | spec §5.5 |
| H-003 | Missed haptic deadline | Force output is stale | Surgeon loses feedback, tissue damage | spec §4.2 |
| H-004 | Tool enters a virtual-fixture zone | SDF miscalculation or latency | Injury to nerve or artery | spec §4.3 |
| H-005 | Network latency spike | Delayed or dropped commands | Motion sickness, loss of control | spec §4.6 |
| H-006 | Single-channel safety computation error | Safety monitor reports incorrectly | Failure to stop | spec §4.1 |
| H-007 | Audit record lost or altered | Regulatory evidence invalid | Device approval invalidated | spec §4.4 |
| H-008 | PHI disclosed | Log or training data leaks patient data | Privacy harm | spec §3 (HIPAA) |
| H-009 | Wrong medication dispensed | Pharmacy verification fails | Patient harm | spec §5.3 |
| H-010 | Narcotic diversion | Chain-of-custody spoofed | Patient and staff harm | spec §5.2 |
| H-011 | Cross-contamination | Robot re-enters clean zone unsterilized | Infection | spec §5.2 |
| H-012 | Inadequate UV-C dose | Surface not fully treated | Infection | spec §5.4 |
| H-013 | Power loss during procedure | Arm drops or moves unexpectedly | Tissue damage | spec §5.5 |

## 3. Risk analysis and evaluation

| Hazard | Severity (1–5) | Probability (1–5) | Initial risk | Controls | Residual risk |
|---|---|---|---|---|---|
| H-001 | `[ ]` | `[ ]` | `[ ]` | TMR workspace check, `tpt-t-medical-safety` | `[ ]` |
| H-002 | `[ ]` | `[ ]` | `[ ]` | Force limiter, immediate stop | `[ ]` |
| H-003 | `[ ]` | `[ ]` | `[ ]` | Hold-last-value, deadline monitoring | `[ ]` |
| H-004 | `[ ]` | `[ ]` | `[ ]` | SDF fixture, progressive force, override | `[ ]` |
| H-005 | `[ ]` | `[ ]` | `[ ]` | Predictive display, jitter masking | `[ ]` |
| H-006 | `[ ]` | `[ ]` | `[ ]` | Three-core TMR voter | `[ ]` |
| H-007 | `[ ]` | `[ ]` | `[ ]` | Signed, hash-chained WAL | `[ ]` |
| H-008 | `[ ]` | `[ ]` | `[ ]` | AES-256-GCM, consent tokens | `[ ]` |
| H-009 | `[ ]` | `[ ]` | `[ ]` | Barcode and weight verification | `[ ]` |
| H-010 | `[ ]` | `[ ]` | `[ ]` | Biometric or badge auth, signed log | `[ ]` |
| H-011 | `[ ]` | `[ ]` | `[ ]` | Mandatory UV sterilization before clean zone | `[ ]` |
| H-012 | `[ ]` | `[ ]` | `[ ]` | Post-cycle re-scan, coverage validation | `[ ]` |
| H-013 | `[ ]` | `[ ]` | `[ ]` | Brakes engaged or safe-pose retraction | `[ ]` |

## 4. Risk control measures

For each control, record:

- Control ID (`RC-###`)
- Hazards addressed
- Implementation (crate and module)
- Verification evidence (test ID, see the traceability matrix)
- Residual-risk rating after verification

## 5. Overall residual risk

`[Summary once all controls are verified: acceptable or not, and why.]`

## 6. Production and post-production

- Post-market surveillance: `[define]`.
- Audit-log export as a field-data source: see spec §6, "Post-Op".

## 7. Revision history

| Version | Date | Author | Change |
|---|---|---|---|
| 0.1-draft | `[ ]` | `[ ]` | Initial template with seeded hazards |
