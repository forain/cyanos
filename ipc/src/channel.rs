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
    ///
    /// Returns `None` if the port table is full.  On partial failure (first
    /// port allocated but second fails) the first port is closed so no slot
    /// is leaked.
    pub fn new(pid: u32) -> Option<Self> {
        let client = port::create(pid)?;
        let server = match port::create(pid) {
            Some(p) => p,
            None    => { port::close(client); return None; }
        };
        Some(Self { client, server })
    }
}
