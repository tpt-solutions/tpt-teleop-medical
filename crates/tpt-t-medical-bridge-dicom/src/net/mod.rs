//! DICOM network services (PS3.7, PS3.8): association setup, DIMSE commands,
//! and the service class user (SCU) operations used for pre-op retrieval.
//!
//! * [`pdu`] encodes and decodes the upper-layer PDUs, over any stream.
//! * [`dimse`] encodes the command sets of C-ECHO, C-FIND, C-GET and C-STORE.
//! * [`dataset`] encodes and decodes Implicit and Explicit VR data elements.
//! * [`client::Association`] runs the operations as the calling side.
//!
//! Network use is bounded. A PDU above 1 MiB, or a message above 256 MiB, is
//! refused. The listening side (SCP) is not implemented, and the tests use a
//! small one on loopback.

pub mod client;
pub mod dataset;
pub mod dimse;
pub mod pdu;

pub use client::{Association, NetError, Retrieved};
