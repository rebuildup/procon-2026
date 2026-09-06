//! Envelope / protocol version types.

use serde::{Deserialize, Serialize};

/// Protocol version. Both peers must agree on `major`; `minor` is informational.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

/// Top-level envelope. Real messages carry their kind + payload inline; this is
/// reserved for future framing (e.g. WebSocket binary frames with compression).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Envelope {
    pub protocol: ProtocolVersion,
    pub kind: String,
    pub body: serde_json::Value,
}
