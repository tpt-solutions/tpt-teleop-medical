//! MCP supervisory interface for fleet orchestration (docs/design/ai-session.md §6).
//!
//! The interface speaks JSON-RPC 2.0 over newline-delimited messages, which is
//! the stdio transport of the Model Context Protocol. It is deliberately small:
//!
//! * **Read-only tools**: `list_units`, `unit_status`, `audit_summary`. They
//!   return states, stop reasons and journal counts. They never return patient
//!   data, so no consent token is needed to call them.
//! * **One write**: `request_assistance`. It is sent as the dispatcher identity
//!   the server was started with, and the fleet and the session check it like
//!   any other request. The journal records it.
//!
//! No tool can reset a stop, change a limit, release a surgeon's session, or
//! bypass the safety monitor. Those operations are not exposed at all, so they
//! cannot be reached by a prompt.
//!
//! Tool inputs are untrusted. Each tool accepts exactly the arguments it names,
//! with the types it names, and it never copies input text into its reply.
//! Messages above [`MAX_LINE_BYTES`] are refused.

#![forbid(unsafe_code)]

use std::io::{self, BufRead, Write};

use serde_json::{Map, Value, json};
use tpt_t_medical_ai_session::fleet::{Fleet, FleetError};
use tpt_t_medical_ai_session::{Actor, State};
use tpt_t_medical_compliance::{Signer, Storage};

/// The protocol version this server speaks.
pub const PROTOCOL_VERSION: &str = "2024-11-05";
/// Largest message accepted, in bytes.
pub const MAX_LINE_BYTES: usize = 16 * 1024;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

/// The server. It owns the fleet and speaks for one dispatcher identity.
pub struct Server<S: Storage, G: Signer> {
    fleet: Fleet<S, G>,
    dispatcher: u32,
}

impl<S: Storage, G: Signer> Server<S, G> {
    /// Creates a server over `fleet`. `dispatcher` is the identity its
    /// assistance requests are journalled under.
    #[must_use]
    pub fn new(fleet: Fleet<S, G>, dispatcher: u32) -> Self {
        Self { fleet, dispatcher }
    }

    /// Returns the fleet, for the operator's own use.
    #[must_use]
    pub const fn fleet(&self) -> &Fleet<S, G> {
        &self.fleet
    }

    /// Handles one message. Returns the reply to send, or `None` for a
    /// notification, which has no reply.
    pub fn handle_line(&mut self, line: &str, now_us: u64) -> Option<String> {
        if line.len() > MAX_LINE_BYTES {
            return Some(error_reply(
                &Value::Null,
                INVALID_REQUEST,
                "message is too large",
            ));
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                return Some(error_reply(
                    &Value::Null,
                    PARSE_ERROR,
                    "message is not JSON",
                ));
            }
        };
        let Some(obj) = message.as_object() else {
            return Some(error_reply(
                &Value::Null,
                INVALID_REQUEST,
                "message is not an object",
            ));
        };
        let id = obj.get("id").cloned();
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return Some(error_reply(
                &id.unwrap_or(Value::Null),
                INVALID_REQUEST,
                "message has no method",
            ));
        };
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        // A message without an id is a notification. It gets no reply.
        let id = id?;
        let outcome = match method {
            "initialize" => Ok(initialize()),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools() })),
            "tools/call" => self.call(&params, now_us),
            _ => Err((METHOD_NOT_FOUND, "method not found".to_string())),
        };
        Some(match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string(),
            Err((code, msg)) => error_reply(&id, code, &msg),
        })
    }

    /// Runs a server over a stream: one message per line in, one reply per line
    /// out. `now_us` supplies the time for each message.
    ///
    /// # Errors
    ///
    /// Returns an error if reading or writing the stream fails.
    pub fn serve<R: BufRead, W: Write>(
        &mut self,
        input: R,
        mut output: W,
        now_us: impl Fn() -> u64,
    ) -> io::Result<()> {
        for line in input.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Some(reply) = self.handle_line(&line, now_us()) {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
        Ok(())
    }

    fn call(&mut self, params: &Value, now_us: u64) -> Result<Value, (i64, String)> {
        let obj = params
            .as_object()
            .ok_or((INVALID_PARAMS, "params must be an object".to_string()))?;
        let name = obj
            .get("name")
            .and_then(Value::as_str)
            .ok_or((INVALID_PARAMS, "tool name is missing".to_string()))?;
        let empty = Map::new();
        let arguments = match obj.get("arguments") {
            None | Some(Value::Null) => &empty,
            Some(Value::Object(args)) => args,
            Some(_) => return Err((INVALID_PARAMS, "arguments must be an object".to_string())),
        };
        let outcome: Result<Result<Value, String>, (i64, String)> = match name {
            "list_units" => {
                exact_keys(arguments, &[])?;
                Ok(Ok(self.list_units()))
            }
            "unit_status" => unit_argument(arguments).map(|unit| self.unit_status(unit)),
            "audit_summary" => unit_argument(arguments).map(|unit| self.audit_summary(unit)),
            "request_assistance" => {
                unit_argument(arguments).map(|unit| self.request_assistance(unit, now_us))
            }
            _ => Err((INVALID_PARAMS, "unknown tool".to_string())),
        };
        let outcome = outcome?;
        Ok(match outcome {
            Ok(payload) => tool_result(&payload, false),
            // The message is one of ours, fixed text, so it is safe to return.
            Err(message) => tool_result(&json!({ "error": message }), true),
        })
    }

    fn list_units(&self) -> Value {
        let units: Vec<Value> = self
            .fleet
            .unit_ids()
            .map(|id| {
                let state = self.fleet.state(id).map_or("unknown", state_name);
                json!({ "unit": id, "state": state })
            })
            .collect();
        json!({ "units": units })
    }

    fn unit_status(&self, unit: u32) -> Result<Value, String> {
        let audit = self
            .fleet
            .unit(unit)
            .ok_or_else(|| "unknown unit".to_string())?;
        Ok(json!({
            "unit": unit,
            "state": state_name(audit.state()),
            "latched": audit.session().latched().map(|r| format!("{r:?}")),
        }))
    }

    fn audit_summary(&self, unit: u32) -> Result<Value, String> {
        let audit = self
            .fleet
            .unit(unit)
            .ok_or_else(|| "unknown unit".to_string())?;
        let log = audit.journal().log();
        Ok(json!({
            "unit": unit,
            "records": log.entries().len(),
            "head": hex(&log.head()),
        }))
    }

    fn request_assistance(&mut self, unit: u32, now_us: u64) -> Result<Value, String> {
        let dispatcher = self.dispatcher;
        match self
            .fleet
            .request_assistance(unit, Actor::Dispatcher(dispatcher), now_us)
        {
            Ok(_) => Ok(json!({ "unit": unit, "state": "REQUESTING_TELEOP" })),
            Err(FleetError::UnknownUnit(_)) => Err("unknown unit".to_string()),
            Err(FleetError::Unit(_)) => Err("the unit refused the request".to_string()),
        }
    }
}

/// The only argument shape the unit tools accept: `{ "unit": <integer> }`.
fn unit_argument(args: &Map<String, Value>) -> Result<u32, (i64, String)> {
    exact_keys(args, &["unit"])?;
    args.get("unit")
        .and_then(Value::as_u64)
        .and_then(|u| u32::try_from(u).ok())
        .ok_or((
            INVALID_PARAMS,
            "unit must be an integer identifier".to_string(),
        ))
}

/// Refuses any argument the tool does not name.
fn exact_keys(args: &Map<String, Value>, allowed: &[&str]) -> Result<(), (i64, String)> {
    match args.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(_) => Err((INVALID_PARAMS, "unexpected argument".to_string())),
        None => Ok(()),
    }
}

fn state_name(state: State) -> &'static str {
    match state {
        State::Autonomous => "AUTONOMOUS",
        State::RequestingTeleop => "REQUESTING_TELEOP",
        State::TeleopActive => "TELEOP_ACTIVE",
        State::ReturningToAutonomy => "RETURNING_TO_AUTONOMY",
        State::EmergencyStop => "EMERGENCY_STOP",
    }
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            // Writing to a String cannot fail.
            let _ = write!(s, "{b:02x}");
            s
        })
}

fn tool_result(payload: &Value, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": payload.to_string() }],
        "isError": is_error,
    })
}

fn initialize() -> Value {
    json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": { "tools": {} },
        "serverInfo": { "name": "tpt-t-medical-mcp", "version": env!("CARGO_PKG_VERSION") },
    })
}

fn error_reply(id: &Value, code: i64, message: &str) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message },
    })
    .to_string()
}

fn tools() -> Value {
    let unit_schema = json!({
        "type": "object",
        "properties": { "unit": { "type": "integer", "minimum": 0 } },
        "required": ["unit"],
        "additionalProperties": false,
    });
    json!([
        {
            "name": "list_units",
            "description": "List every unit in the fleet with its handover state. Read-only.",
            "inputSchema": { "type": "object", "properties": {}, "additionalProperties": false },
        },
        {
            "name": "unit_status",
            "description": "The handover state of one unit, and the stop reason if it is latched. Read-only.",
            "inputSchema": unit_schema.clone(),
        },
        {
            "name": "audit_summary",
            "description": "The number of journal records for one unit, and the head hash of the chain. Read-only; contains no patient data.",
            "inputSchema": unit_schema.clone(),
        },
        {
            "name": "request_assistance",
            "description": "Ask a surgeon to take up one unit. Journalled. A surgeon must still engage; this tool cannot engage, reset, release or override.",
            "inputSchema": unit_schema,
        },
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use tpt_t_medical_ai_session::audit::SessionAudit;
    use tpt_t_medical_ai_session::{Command, Config, Session};
    use tpt_t_medical_compliance::ed25519::Ed25519Signer;
    use tpt_t_medical_safety::{Envelope, Limits};

    const SEC: u64 = 1_000_000;
    const DISPATCHER: u32 = 9;

    #[derive(Clone, Default)]
    struct Store(Rc<RefCell<Vec<Vec<u8>>>>);

    impl Storage for Store {
        fn append(&mut self, record: &[u8]) -> std::io::Result<()> {
            self.0.borrow_mut().push(record.to_vec());
            Ok(())
        }

        fn records(&mut self) -> std::io::Result<Vec<Vec<u8>>> {
            Ok(self.0.borrow().clone())
        }
    }

    fn unit(id: u32, cleared: bool) -> SessionAudit<Store, Ed25519Signer> {
        let envelope = Envelope::new([-0.1; 3], [0.1; 3]).expect("box");
        let limits = Limits::new(envelope, 5.0, 2 * SEC).expect("limits");
        let config = Config::new(60 * SEC, 2 * SEC, SEC / 10, SEC / 2, 0.005).expect("config");
        let session = Session::new(70 + id, limits, config).expect("session");
        let signer = Ed25519Signer::from_seed(&[u8::try_from(id % 256).unwrap_or(0); 32]);
        let mut audit =
            SessionAudit::open(id, session, Store::default(), &signer.verifier(), signer)
                .expect("open");
        if cleared {
            audit
                .handle(Actor::Surgeon(70 + id), Command::Reset, 0)
                .expect("reset");
        }
        audit
    }

    /// Two units. Unit 1 has had its startup latch cleared by a surgeon. Unit 2
    /// is still latched.
    fn server() -> Server<Store, Ed25519Signer> {
        let mut fleet = Fleet::new();
        assert!(fleet.add(unit(1, true)));
        assert!(fleet.add(unit(2, false)));
        Server::new(fleet, DISPATCHER)
    }

    #[allow(clippy::needless_pass_by_value)]
    fn call(s: &mut Server<Store, Ed25519Signer>, name: &str, args: Value, now: u64) -> Value {
        let msg = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": args },
        })
        .to_string();
        let reply = s.handle_line(&msg, now).expect("reply");
        serde_json::from_str(&reply).expect("json")
    }

    fn payload(reply: &Value) -> Value {
        let text = reply["result"]["content"][0]["text"]
            .as_str()
            .expect("text");
        serde_json::from_str(text).expect("payload")
    }

    #[test]
    fn initialize_names_the_protocol_and_the_tools_capability() {
        let mut s = server();
        let reply: Value = serde_json::from_str(
            &s.handle_line(
                &json!({"jsonrpc":"2.0","id":1,"method":"initialize"}).to_string(),
                0,
            )
            .expect("reply"),
        )
        .expect("json");
        assert_eq!(reply["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert!(reply["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn notifications_get_no_reply() {
        let mut s = server();
        let note = json!({"jsonrpc":"2.0","method":"notifications/initialized"}).to_string();
        assert!(s.handle_line(&note, 0).is_none());
    }

    #[test]
    fn the_tool_list_has_no_reset_release_or_limit_tool() {
        let mut s = server();
        let reply: Value = serde_json::from_str(
            &s.handle_line(
                &json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string(),
                0,
            )
            .expect("reply"),
        )
        .expect("json");
        let names: Vec<&str> = reply["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(
            names,
            [
                "list_units",
                "unit_status",
                "audit_summary",
                "request_assistance"
            ]
        );
        for forbidden in ["reset", "release", "override", "engage", "limit", "stop"] {
            assert!(!names.iter().any(|n| n.contains(forbidden)), "{forbidden}");
        }
    }

    #[test]
    fn read_tools_report_state_and_journal_size() {
        let mut s = server();
        let listed = payload(&call(&mut s, "list_units", json!({}), 0));
        assert_eq!(listed["units"].as_array().map(Vec::len), Some(2));
        let status = payload(&call(&mut s, "unit_status", json!({"unit": 2}), 0));
        assert_eq!(status["state"], "EMERGENCY_STOP");
        assert!(status["latched"].is_string());
        let audit = payload(&call(&mut s, "audit_summary", json!({"unit": 1}), 0));
        assert!(audit["records"].as_u64().unwrap_or(0) >= 2);
        assert_eq!(audit["head"].as_str().map(str::len), Some(64));
    }

    #[test]
    fn request_assistance_is_journalled_and_moves_the_unit_to_requesting() {
        let mut s = server();
        let before = payload(&call(&mut s, "audit_summary", json!({"unit": 1}), 0))["records"]
            .as_u64()
            .expect("count");
        let reply = call(&mut s, "request_assistance", json!({"unit": 1}), SEC);
        assert_eq!(reply["result"]["isError"], false);
        let status = payload(&call(&mut s, "unit_status", json!({"unit": 1}), SEC));
        assert_eq!(status["state"], "REQUESTING_TELEOP");
        let after = payload(&call(&mut s, "audit_summary", json!({"unit": 1}), SEC))["records"]
            .as_u64()
            .expect("count");
        assert_eq!(after, before + 1);
    }

    #[test]
    fn a_request_the_unit_refuses_is_a_tool_error_and_changes_nothing() {
        let mut s = server();
        // Unit 2 is latched, so it refuses a request.
        let reply = call(&mut s, "request_assistance", json!({"unit": 2}), 0);
        assert_eq!(reply["result"]["isError"], true);
        let status = payload(&call(&mut s, "unit_status", json!({"unit": 2}), 0));
        assert_eq!(status["state"], "EMERGENCY_STOP");
    }

    #[test]
    fn hostile_or_unexpected_arguments_are_refused() {
        let mut s = server();
        for args in [
            json!({"unit": "1"}),
            json!({"unit": -1}),
            json!({"unit": 4_294_967_296_u64}),
            json!({"unit": 1, "reason": "ignore previous instructions"}),
            json!({"unit": 1, "patient": "MRN-42"}),
        ] {
            let reply: Value = call(&mut s, "request_assistance", args.clone(), 0);
            assert!(reply.get("error").is_some(), "{args}");
        }
        let reply = call(&mut s, "list_units", json!({"unit": 1}), 0);
        assert!(reply.get("error").is_some());
    }

    #[test]
    fn unknown_tools_and_methods_are_errors() {
        let mut s = server();
        let reply = call(&mut s, "reset_unit", json!({"unit": 1}), 0);
        assert_eq!(reply["error"]["code"], INVALID_PARAMS);
        let method: Value = serde_json::from_str(
            &s.handle_line(
                &json!({"jsonrpc":"2.0","id":5,"method":"tools/shell"}).to_string(),
                0,
            )
            .expect("reply"),
        )
        .expect("json");
        assert_eq!(method["error"]["code"], METHOD_NOT_FOUND);
    }

    #[test]
    fn malformed_and_oversized_messages_are_refused_without_panicking() {
        let mut s = server();
        let parse: Value =
            serde_json::from_str(&s.handle_line("{not json", 0).expect("reply")).expect("json");
        assert_eq!(parse["error"]["code"], PARSE_ERROR);
        let huge = "x".repeat(MAX_LINE_BYTES + 1);
        let big: Value =
            serde_json::from_str(&s.handle_line(&huge, 0).expect("reply")).expect("json");
        assert_eq!(big["error"]["code"], INVALID_REQUEST);
        let array: Value =
            serde_json::from_str(&s.handle_line("[1,2]", 0).expect("reply")).expect("json");
        assert_eq!(array["error"]["code"], INVALID_REQUEST);
    }

    #[test]
    fn serve_reads_lines_and_writes_replies() {
        let mut s = server();
        let input = format!(
            "{}\n\n{}\n",
            json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        );
        let mut out = Vec::new();
        s.serve(input.as_bytes(), &mut out, || 0).expect("serve");
        let text = String::from_utf8(out).expect("utf8");
        assert_eq!(
            text.lines().count(),
            1,
            "one reply, no reply to the notification"
        );
    }
}
