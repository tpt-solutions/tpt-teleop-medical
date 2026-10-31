//! Elevator requests over `MQTT 3.1.1` (spec.txt §5.2).
//!
//! The framing is written here, not taken from a client crate, so the workspace
//! keeps its licence policy and adds no dependency. Only the `PUBLISH` packet at
//! `QoS 0` is supported. `QoS 1 and 2` need acknowledgements that this module does
//! not send, so they are refused rather than half-implemented. Retained messages
//! are refused for the same reason. A broker connection is not made here. The
//! caller carries the bytes.
//!
//! The elevator protocol is three topics:
//!
//! * `elevator/request`, payload `robot=<id> emergency=<0|1>`. A robot asks for
//!   the cab.
//! * `elevator/grant`, payload `robot=<id> grant=<result>`. The answer, with
//!   `boarded`, `queued:<n>`, `aboard`, or `queued-already:<n>`.
//! * `elevator/release`, payload `robot=<id>`. The robot has left the cab. The
//!   next robot in the queue, if any, is announced on `elevator/boarded`.

use core::fmt;

use crate::elevator::{Elevator, Grant, NotOccupant};

/// Largest topic accepted, in bytes.
pub const MAX_TOPIC_BYTES: usize = 256;
/// Largest payload accepted, in bytes.
pub const MAX_PAYLOAD_BYTES: usize = 1024;

const PUBLISH_QOS0: u8 = 0x30;
const TOPIC_REQUEST: &str = "elevator/request";
const TOPIC_GRANT: &str = "elevator/grant";
const TOPIC_RELEASE: &str = "elevator/release";
const TOPIC_BOARDED: &str = "elevator/boarded";

/// One application message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The topic name.
    pub topic: String,
    /// The payload.
    pub payload: Vec<u8>,
}

/// Why a packet or message was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The bytes end before the packet does.
    Truncated,
    /// The packet is not a `PUBLISH` at `QoS 0` without the retain flag.
    Unsupported,
    /// The remaining-length field is malformed or too long.
    BadLength,
    /// The topic is empty, not UTF-8, or contains a wildcard or NUL.
    BadTopic,
    /// The topic is longer than [`MAX_TOPIC_BYTES`].
    TopicTooLong,
    /// The payload is longer than [`MAX_PAYLOAD_BYTES`].
    PayloadTooLarge,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("packet is truncated"),
            Self::Unsupported => f.write_str("only QoS 0 PUBLISH without retain is supported"),
            Self::BadLength => f.write_str("remaining length is malformed"),
            Self::BadTopic => f.write_str("topic is not a valid topic name"),
            Self::TopicTooLong => f.write_str("topic is above the limit"),
            Self::PayloadTooLarge => f.write_str("payload is above the limit"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Encodes one message as a `QoS 0` `PUBLISH` packet.
///
/// # Errors
///
/// Returns an error if the topic or payload is outside the limits.
pub fn encode(message: &Message) -> Result<Vec<u8>, FrameError> {
    check_topic(&message.topic)?;
    if message.payload.len() > MAX_PAYLOAD_BYTES {
        return Err(FrameError::PayloadTooLarge);
    }
    let topic = message.topic.as_bytes();
    let remaining = 2 + topic.len() + message.payload.len();
    let mut out = vec![PUBLISH_QOS0];
    encode_length(remaining, &mut out);
    let topic_len = u16::try_from(topic.len()).map_err(|_| FrameError::TopicTooLong)?;
    out.extend_from_slice(&topic_len.to_be_bytes());
    out.extend_from_slice(topic);
    out.extend_from_slice(&message.payload);
    Ok(out)
}

/// Decodes one `QoS 0` `PUBLISH` packet. The packet must be exactly the input: any
/// trailing byte is refused.
///
/// # Errors
///
/// Returns the first problem found.
pub fn decode(packet: &[u8]) -> Result<Message, FrameError> {
    let (&first, rest) = packet.split_first().ok_or(FrameError::Truncated)?;
    if first != PUBLISH_QOS0 {
        return Err(FrameError::Unsupported);
    }
    let (remaining, body) = decode_length(rest)?;
    if body.len() != remaining {
        return Err(FrameError::BadLength);
    }
    let topic_len_bytes: [u8; 2] = body
        .get(..2)
        .and_then(|b| b.try_into().ok())
        .ok_or(FrameError::Truncated)?;
    let topic_len = usize::from(u16::from_be_bytes(topic_len_bytes));
    if topic_len > MAX_TOPIC_BYTES {
        return Err(FrameError::TopicTooLong);
    }
    let topic_end = 2 + topic_len;
    let topic_bytes = body.get(2..topic_end).ok_or(FrameError::Truncated)?;
    let topic = std::str::from_utf8(topic_bytes).map_err(|_| FrameError::BadTopic)?;
    check_topic(topic)?;
    let payload = &body[topic_end..];
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(FrameError::PayloadTooLarge);
    }
    Ok(Message {
        topic: topic.to_string(),
        payload: payload.to_vec(),
    })
}

fn check_topic(topic: &str) -> Result<(), FrameError> {
    if topic.len() > MAX_TOPIC_BYTES {
        return Err(FrameError::TopicTooLong);
    }
    // A topic name in a PUBLISH may not hold wildcards or NUL.
    if topic.is_empty() || topic.contains(['+', '#', '\0']) {
        return Err(FrameError::BadTopic);
    }
    Ok(())
}

/// Writes MQTT's variable-length integer.
fn encode_length(mut value: usize, out: &mut Vec<u8>) {
    loop {
        let mut byte = u8::try_from(value % 128).unwrap_or(0);
        value /= 128;
        if value > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Reads MQTT's variable-length integer, at most four bytes long.
fn decode_length(bytes: &[u8]) -> Result<(usize, &[u8]), FrameError> {
    let mut value = 0_usize;
    let mut multiplier = 1_usize;
    for (i, &byte) in bytes.iter().take(4).enumerate() {
        value += usize::from(byte & 0x7f) * multiplier;
        if byte & 0x80 == 0 {
            return Ok((value, &bytes[i + 1..]));
        }
        multiplier *= 128;
    }
    if bytes.len() < 4 {
        Err(FrameError::Truncated)
    } else {
        Err(FrameError::BadLength)
    }
}

/// Why an elevator message was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevatorMessageError {
    /// The packet itself is not valid.
    Frame(FrameError),
    /// The topic is not one the elevator service handles.
    UnknownTopic,
    /// The payload does not match the topic's format.
    BadPayload,
    /// The robot is not in the cab.
    NotOccupant,
}

impl fmt::Display for ElevatorMessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(e) => write!(f, "{e}"),
            Self::UnknownTopic => f.write_str("topic is not an elevator topic"),
            Self::BadPayload => f.write_str("payload does not match the topic"),
            Self::NotOccupant => f.write_str("robot is not in the elevator"),
        }
    }
}

impl std::error::Error for ElevatorMessageError {}

impl From<FrameError> for ElevatorMessageError {
    fn from(e: FrameError) -> Self {
        Self::Frame(e)
    }
}

/// Serves elevator requests from encoded MQTT packets. It owns the cab.
#[derive(Debug, Default)]
pub struct ElevatorService {
    elevator: Elevator,
}

impl ElevatorService {
    /// Creates a service with an empty cab.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            elevator: Elevator::new(),
        }
    }

    /// Handles one incoming packet and returns the packets to publish in reply.
    /// A refused packet changes nothing.
    ///
    /// # Errors
    ///
    /// Returns an error if the packet or its payload is not valid, or if a
    /// release is for a robot that is not in the cab.
    pub fn handle(&mut self, packet: &[u8]) -> Result<Vec<Vec<u8>>, ElevatorMessageError> {
        let msg = decode(packet)?;
        let replies = match msg.topic.as_str() {
            TOPIC_REQUEST => {
                let (robot, emergency) = parse_request(&msg.payload)?;
                let grant = self.elevator.request(robot, emergency);
                vec![reply(
                    TOPIC_GRANT,
                    format!("robot={robot} grant={}", grant_text(grant)),
                )?]
            }
            TOPIC_RELEASE => {
                let robot = parse_robot(&msg.payload)?;
                let next = self
                    .elevator
                    .release(robot)
                    .map_err(|NotOccupant| ElevatorMessageError::NotOccupant)?;
                match next {
                    Some(next) => vec![reply(TOPIC_BOARDED, format!("robot={next}"))?],
                    None => Vec::new(),
                }
            }
            _ => return Err(ElevatorMessageError::UnknownTopic),
        };
        Ok(replies)
    }
}

fn reply(topic: &str, payload: String) -> Result<Vec<u8>, ElevatorMessageError> {
    Ok(encode(&Message {
        topic: topic.to_string(),
        payload: payload.into_bytes(),
    })?)
}

fn grant_text(grant: Grant) -> String {
    match grant {
        Grant::Boarded => "boarded".to_string(),
        Grant::Queued { position } => format!("queued:{position}"),
        Grant::AlreadyAboard => "aboard".to_string(),
        Grant::AlreadyQueued { position } => format!("queued-already:{position}"),
    }
}

fn parse_robot(payload: &[u8]) -> Result<u32, ElevatorMessageError> {
    let text = std::str::from_utf8(payload).map_err(|_| ElevatorMessageError::BadPayload)?;
    text.strip_prefix("robot=")
        .and_then(|id| id.parse().ok())
        .ok_or(ElevatorMessageError::BadPayload)
}

fn parse_request(payload: &[u8]) -> Result<(u32, bool), ElevatorMessageError> {
    let text = std::str::from_utf8(payload).map_err(|_| ElevatorMessageError::BadPayload)?;
    let (robot_part, emergency_part) = text
        .split_once(' ')
        .ok_or(ElevatorMessageError::BadPayload)?;
    let robot = parse_robot(robot_part.as_bytes())?;
    let emergency = match emergency_part {
        "emergency=0" => false,
        "emergency=1" => true,
        _ => return Err(ElevatorMessageError::BadPayload),
    };
    Ok((robot, emergency))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn publish(topic: &str, payload: &str) -> Vec<u8> {
        encode(&Message {
            topic: topic.into(),
            payload: payload.as_bytes().to_vec(),
        })
        .expect("encode")
    }

    #[test]
    fn a_message_round_trips() {
        let msg = Message {
            topic: "elevator/request".into(),
            payload: b"robot=4 emergency=1".to_vec(),
        };
        let bytes = encode(&msg).expect("encode");
        assert_eq!(bytes[0], 0x30);
        assert_eq!(decode(&bytes), Ok(msg));
    }

    #[test]
    fn long_payloads_use_a_two_byte_remaining_length() {
        let msg = Message {
            topic: "t".into(),
            payload: vec![7; 300],
        };
        let bytes = encode(&msg).expect("encode");
        assert_eq!(decode(&bytes), Ok(msg));
    }

    #[test]
    fn qos_retain_and_other_packet_types_are_refused() {
        let good = publish("a", "b");
        for first in [0x32_u8, 0x34, 0x31, 0x10, 0xc0] {
            let mut bad = good.clone();
            bad[0] = first;
            assert_eq!(decode(&bad), Err(FrameError::Unsupported), "{first:#x}");
        }
    }

    #[test]
    fn wildcards_are_refused_in_topic_names() {
        assert_eq!(
            encode(&Message {
                topic: "elevator/+".into(),
                payload: vec![]
            }),
            Err(FrameError::BadTopic)
        );
        assert_eq!(
            encode(&Message {
                topic: String::new(),
                payload: vec![]
            }),
            Err(FrameError::BadTopic)
        );
    }

    #[test]
    fn trailing_or_missing_bytes_are_refused() {
        let mut extra = publish("a", "b");
        extra.push(0);
        assert_eq!(decode(&extra), Err(FrameError::BadLength));
        let short = publish("a", "bcd");
        assert_eq!(
            decode(&short[..short.len() - 1]),
            Err(FrameError::BadLength)
        );
        assert_eq!(decode(&[]), Err(FrameError::Truncated));
    }

    #[test]
    fn oversized_fields_are_refused_on_both_sides() {
        let long_topic = "x".repeat(MAX_TOPIC_BYTES + 1);
        assert_eq!(
            encode(&Message {
                topic: long_topic,
                payload: vec![]
            }),
            Err(FrameError::TopicTooLong)
        );
        assert_eq!(
            encode(&Message {
                topic: "a".into(),
                payload: vec![0; MAX_PAYLOAD_BYTES + 1]
            }),
            Err(FrameError::PayloadTooLarge)
        );
    }

    #[test]
    fn a_runaway_length_field_is_refused() {
        assert_eq!(
            decode(&[0x30, 0xff, 0xff, 0xff, 0xff, 0x01]),
            Err(FrameError::BadLength)
        );
    }

    #[test]
    fn request_boards_then_queues_then_release_announces_the_next() {
        let mut svc = ElevatorService::new();
        let r1 = svc
            .handle(&publish(TOPIC_REQUEST, "robot=1 emergency=0"))
            .expect("r1");
        assert_eq!(
            decode(&r1[0]).expect("frame").payload,
            b"robot=1 grant=boarded".to_vec()
        );
        let r2 = svc
            .handle(&publish(TOPIC_REQUEST, "robot=2 emergency=0"))
            .expect("r2");
        assert_eq!(
            decode(&r2[0]).expect("frame").payload,
            b"robot=2 grant=queued:0".to_vec()
        );
        let released = svc
            .handle(&publish(TOPIC_RELEASE, "robot=1"))
            .expect("release");
        let announced = decode(&released[0]).expect("frame");
        assert_eq!(announced.topic, TOPIC_BOARDED);
        assert_eq!(announced.payload, b"robot=2".to_vec());
    }

    #[test]
    fn an_emergency_request_jumps_the_queue() {
        let mut svc = ElevatorService::new();
        svc.handle(&publish(TOPIC_REQUEST, "robot=1 emergency=0"))
            .expect("r1");
        svc.handle(&publish(TOPIC_REQUEST, "robot=2 emergency=0"))
            .expect("r2");
        svc.handle(&publish(TOPIC_REQUEST, "robot=3 emergency=1"))
            .expect("r3");
        let released = svc
            .handle(&publish(TOPIC_RELEASE, "robot=1"))
            .expect("release");
        assert_eq!(
            decode(&released[0]).expect("frame").payload,
            b"robot=3".to_vec()
        );
    }

    #[test]
    fn malformed_requests_and_wrong_topics_change_nothing() {
        let mut svc = ElevatorService::new();
        for payload in ["robot=1", "robot=x emergency=0", "robot=1 emergency=2", ""] {
            assert_eq!(
                svc.handle(&publish(TOPIC_REQUEST, payload)),
                Err(ElevatorMessageError::BadPayload),
                "{payload}"
            );
        }
        assert_eq!(
            svc.handle(&publish("hospital/other", "x")),
            Err(ElevatorMessageError::UnknownTopic)
        );
        assert_eq!(
            svc.handle(&publish(TOPIC_RELEASE, "robot=9")),
            Err(ElevatorMessageError::NotOccupant)
        );
        // The cab is still empty, so the next robot boards at once.
        let r = svc
            .handle(&publish(TOPIC_REQUEST, "robot=5 emergency=0"))
            .expect("r");
        assert_eq!(
            decode(&r[0]).expect("frame").payload,
            b"robot=5 grant=boarded".to_vec()
        );
    }
}
