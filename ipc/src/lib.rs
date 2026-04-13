//! IPC subsystem — the backbone of the microkernel.
//!
//! Drivers and servers are isolated user-space processes that talk to each
//! other (and to the kernel) exclusively through typed message passing,
//! inspired by L4/seL4 and Linux's socket/pipe model.

#![no_std]

pub mod port;
pub mod message;
pub mod channel;

pub use port::Port;
pub use message::Message;

/// Initialise the IPC subsystem. Called once from `kernel_main`.
pub fn init() {
    port::init();
}
