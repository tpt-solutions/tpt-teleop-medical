# tpt-t-medical-ai-session

The AI controller for tpt-teleop-medical, as an assist-only operator. The AI can
propose motion, and a surgeon decides whether it happens. The design is in
[docs/design/ai-session.md](../../docs/design/ai-session.md).

## Status

Logic done. Wiring to the tpt-teleop domain interface, and a TMR vote on AI
commands, are pending.

## Rules

1. **The AI can only propose.** A proposal is checked by the safety monitor and
   recorded as advice. It never moves the arm. The surgeon's command is the only
   motion source.
2. **Only the operating surgeon has authority.** The surgeon alone can engage,
   release, override, renew or reset. The AI and fleet dispatchers can only
   request assistance.
3. **Sessions expire.** A session ends unless a surgeon renews it. A missed
   heartbeat from the AI is a fault.
4. **Bad proposals latch a stop.** A proposal that leaves the envelope or the force
   limit, or that contradicts the operator's motion, latches an automatic
   emergency stop.
5. **Sessions open latched.** A restart never resumes motion without a surgeon's
   reset.
6. **Everything is journalled.** Every command, refusal and state change is written
   through [`tpt-t-medical-compliance`](../tpt-t-medical-compliance).

Each proposal is checked for the envelope, the force limit, a contradiction with
the operator's motion, and rate.

## States

| `State` | Meaning |
|---|---|
| `Autonomous` | The AI is not controlling and has not been asked to |
| `RequestingTeleop` | Assistance was requested. A surgeon must engage or refuse. |
| `TeleopActive` | The surgeon has engaged. The AI's proposals are advisory. |
| `ReturningToAutonomy` | The surgeon released control. The handover is in progress. |
| `EmergencyStop` | Stopped and latched. Only a surgeon's reset clears it. |

## Main types

| Item | Module | Purpose |
|---|---|---|
| `Session` | root | The state machine. `step`, `state`, `tick`, `handle`. |
| `Command` | root | What is requested. Includes `RequestAssistance`, `Engage`, `Refuse` and the surgeon-only commands. |
| `Actor` | root | `Surgeon(id)`, `Dispatcher(id)` or `Ai`. The caller authenticates the actor. This crate records the identifier and does not check it. |
| `Proposal` | root | One AI proposal, with the readings it was made from |
| `Verdict`, `Refusal` | root | The outcome of a command, and why it was refused |
| `Config` | root | Session configuration, passed to `Session` |
| `SessionAudit` | `audit` | Journals every command for one unit |
| `Fleet` | `fleet` | Several units, each with its own journal. `add`, `tick_all`, `request_assistance`, `unit`, `unit_ids`, `pending` (FIFO queue of units awaiting a surgeon), `summary`. |
| `read_for_ai` | `phi` | The only route by which the AI reads patient data. It goes through the HIPAA consent gate, and logging is refused. |

## Testing

```sh
cargo test -p tpt-t-medical-ai-session
```

## Limitations

- The actor identity is not authenticated here. The caller must do that, as the
  custody ledger does.
- Not yet wired to the tpt-teleop domain interface.
- No TMR vote on AI commands yet.

## Dependencies

- [`tpt-t-medical-compliance`](../tpt-t-medical-compliance)
- [`tpt-t-medical-hipaa`](../tpt-t-medical-hipaa)
- [`tpt-t-medical-safety`](../tpt-t-medical-safety)

## Related crates

- [`tpt-t-medical-mcp`](../tpt-t-medical-mcp) exposes the fleet to dispatchers
  through a read-mostly interface.
- [`tpt-t-medical-sim`](../tpt-t-medical-sim) runs fault scenarios against this
  crate.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
