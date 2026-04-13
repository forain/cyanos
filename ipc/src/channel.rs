//! Bidirectional channel — a paired (client, server) port tuple.
//!
//! Analogous to a Unix socket pair or a Linux pipe.

use super::port::{self, Port};

pub struct Channel {
    pub client: Port,
    pub server: Port,
}

impl Channel {
    /// Create a paired channel for process `pid`.
    pub fn new(pid: u32) -> Option<Self> {
        let client = port::create(pid)?;
        let server = port::create(pid)?;
        Some(Self { client, server })
    }
}
