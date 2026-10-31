# tpt-t-medical-mcp

A Model Context Protocol (MCP) supervisory interface for fleet orchestration in
tpt-teleop-medical (`docs/design/ai-session.md` §6). It lets a dispatcher, human or
AI, see the state of the fleet and ask for assistance. It cannot change safety
state.

## Status

Done. The interface has read-only tools and one journalled write. It has no reset,
release, limit or veto tool, and that is deliberate.

## Design

The interface speaks JSON-RPC 2.0 over newline-delimited messages, the stdio
transport of the Model Context Protocol. Protocol version: `2024-11-05`
(`PROTOCOL_VERSION`).

### Tools

| Tool | Kind | Returns |
|---|---|---|
| `list_units` | read-only | The units in the fleet |
| `unit_status` | read-only | A unit's state and stop reason |
| `audit_summary` | read-only | Journal counts for a unit |
| `request_assistance` | one journalled write | Sends an assistance request to a surgeon |

The read-only tools return states, stop reasons and journal counts. They never
return patient data, so calling them needs no consent token.

`request_assistance` is sent as the dispatcher identity the server was started
with. The fleet and the session check it like any other request, and the journal
records it.

### Not exposed

No tool can reset a stop, change a limit, release a surgeon's session or bypass
the safety monitor. These operations are not in the interface at all, so a prompt
cannot reach them.

### Input handling

Tool inputs are untrusted. Each tool accepts exactly the arguments it names, with
the types it names. The server never copies input text into its reply. Lines above
`MAX_LINE_BYTES` (16 KiB) are refused.

## Main API

| Item | Purpose |
|---|---|
| `Server<S, G>` | The server, over a `Fleet` from `tpt-t-medical-ai-session` |
| `Server::new(fleet, dispatcher)` | Creates a server that sends requests as `dispatcher` |
| `Server::serve(reader, writer)` | Runs the stdio loop |
| `Server::handle_line` | Handles one message. Useful for tests. |
| `Server::fleet` | Read access to the fleet |

## Testing

```sh
cargo test -p tpt-t-medical-mcp
```

## Limitations

- Only the four tools above exist. A dispatcher cannot act on a unit beyond
  requesting assistance.
- Only the stdio transport is provided.

## Dependencies

- [`tpt-t-medical-ai-session`](../tpt-t-medical-ai-session) (the fleet and session)
- [`tpt-t-medical-compliance`](../tpt-t-medical-compliance) (the journal)
- [`tpt-t-medical-safety`](../tpt-t-medical-safety) (stop reasons)
- `serde_json` 1

## Related crates

- [`tpt-t-medical-ai-session`](../tpt-t-medical-ai-session) holds the rules this
  interface depends on.

## Licence

MIT OR Apache-2.0. See the [workspace README](../../README.md).
