# AI session and fleet dispatch: draft design

> **Status: recommendations adopted and implemented in
> `tpt-t-medical-ai-session`.** Open questions 1–4 were settled as this draft
> recommends: assist-only (the AI proposes, the surgeon moves), surgeon-only
> authority, one audit chain per unit, and MCP deferred. Spec §5.6 is still
> not written, so the [assumed] requirements stand until it is. It covers
> Phase 6.5 of [todo.md](../../todo.md). Spec §5.6, "AI & Fleet Dispatch", is referenced by
> the bridge spec but has not been written, so the requirements below are
> inferred from the bridge spec's design notes and from the existing safety
> rules. Each inference is marked **[assumed]** and needs confirmation before
> implementation.

Written for: the people who own the safety and regulatory decisions for this
repository.

## 1. Principles

These follow from the existing design. The AI layer adds no privileges.

1. **An AI controller is a DTI operator.** It takes the `operator_id` slot that a
   human operator takes in the bridge spec (§4.1). Nothing else in the adapter
   changes.
2. **Every command passes the same veto as a human command.** The TMR voter,
   workspace envelope, force limit, watchdog and E-stop in
   `tpt-t-medical-safety` all run on AI commands, with no exemption. An AI
   command is never trusted more than a human one.
3. **Every AI command and every handoff is journalled** through
   `tpt-t-medical-compliance`, before it takes effect. A failed write stops the
   command, as it does for human commands (`tpt-t-medical-core` `AuditedLoop`).
4. **A human can always take control.** Override is immediate and does not wait
   for the AI to yield. It is recorded like any other transition.
5. **AI sessions default to assist-only [assumed].** Full autonomy is an opt-in
   mode. The surgeon must choose it before engaging, for each engagement. It is
   never the default, and it is never chosen by the AI or a dispatcher.
6. **PHI is gated.** An AI or MCP tool sees PHI only with a consent token from
   `tpt-t-medical-hipaa`, and that gate applies to training data too.

## 2. State machine

The bridge spec gives the states. The transitions below are the proposal.

```
AUTONOMOUS ──request──▶ REQUESTING_TELEOP ──engage──▶ TELEOP_ACTIVE
     ▲                        │                            │
     │                        └──────── refuse ──▶ AUTONOMOUS
     │                                                     │
     └────────── RETURNING_TO_AUTONOMY ◀──── release ──────┘
                                  │
   any state ──EMERGENCY_STOP──▶ EMERGENCY_STOP (latched, see §4)
```

- `REQUESTING_TELEOP`: the AI or the domain requests human assistance. A human
  must engage. The AI cannot engage itself.
- `TELEOP_ACTIVE`: in assist mode, the AI's commands are *advisory* and are
  merged with the human's before the veto. In full-autonomy mode (opt-in, see
  principle 5), the AI's checked proposals are the commanded motion, still behind
  the veto. The surgeon can override at any time, and the mode ends with the
  engagement.
- `RETURNING_TO_AUTONOMY`: a smooth handover, using the same hold-last-value
  rules as the haptic loop, so the arm does not jerk.

**Open question 1.** Assist mode blends two sources. The bridge spec says
"continuous blended control is not addressed by this model and is deferred."
The proposal above therefore needs a rule for how the two sources are combined,
or it needs to be deferred too. The simplest safe option is that the AI may only
*propose*, and the human's command is the only one that moves the arm. This
means assist mode gives the AI no motion authority at all, which is the design
this draft recommends.

## 3. Human authorization and override

- Engaging the AI requires an authenticated human action. This is the same kind
  of authentication as the custody ledger (badge or biometric, verified
  upstream).
- Override is a single action, handled at the highest priority: the emergency
  lane of `tpt-t-medical-core`'s event loop. Override does not wait for the
  control loop's budget.
- Each AI session has a bounded lifetime. It expires unless renewed by a human.

**Open question 2.** Who may authorize an AI session: the operating surgeon
only, or also a supervising role? This affects the authorization model and the
audit fields.

## 4. Fault and misbehaviour handling

- A misbehaving AI is detected by the safety monitor, not by the AI itself. The
  monitor's existing checks apply, and a new one is added: a command rate limit
  and a command-to-motion consistency check.
- An AI fault latches an E-stop with source `Automatic`, as an existing safety
  trigger does. Clearing it needs the same operator reset as any other stop.
- The simulator (`tpt-t-medical-sim`) must include fault scenarios for a
  misbehaving AI: commands that exceed limits, commands that stop arriving, and
  commands that contradict the operator.

## 5. Fleet dispatch

- One dispatcher, human or AI, holds many sessions. Each session has its own
  `operator_id`, its own safety monitor, and its own audit chain.
- A dispatcher cannot widen a session's authority. It can only request, and the
  session's own rules decide.
- No cross-session command. A command for unit A is never routed to unit B.

**Open question 3.** Is one audit chain per unit, or one per fleet? Per unit
keeps each device's evidence self-contained. Per fleet makes cross-unit
analysis easier. This draft recommends per unit.

## 6. MCP supervisory interface

- Read-mostly. Reads expose status and audit summaries, with PHI removed unless
  a consent token is presented.
- Writes are a small, named set (for example, request assistance, release a
  session). Each write is authenticated, scoped to one session, journalled, and
  goes through the same gates as a human request.
- No MCP tool can call the safety monitor's reset, change its limits, or bypass
  the veto. Those operations are not exposed at all.
- Tool inputs are untrusted. Free text from a tool call is never interpreted as
  an instruction to the system. This is the defence against prompt injection
  through tool inputs.

**Open question 4.** Is an MCP interface needed for v1 at all? The spec does not
require it, and each tool adds review scope. The draft recommends deferring it
until after the AI session itself has passed review.

## 7. What must be true before implementation

1. Spec §5.6 is written, or the requirements above are confirmed as the
   requirements.
2. Open questions 1–4 are answered.
3. The safety monitor's rate-limit and consistency checks are specified.
4. A security review covers the AI and MCP paths (already listed in todo.md,
   Phase 9).

## 8. What this draft does not do

It does not choose a model, a prompt format, or a vendor. Those are outside the
safety design and can change without affecting the principles in §1.
