//! iLO HTML5 remote console (in-portal).
//!
//! Lets an operator open a machine's iLO console inside TCS without the browser
//! needing to reach the iLO directly (management subnet, self-signed cert,
//! `X-Frame-Options: sameorigin`). TCS logs into the iLO with the stored BMC
//! credentials, serves the console assets from its own origin (rewriting the
//! KVM WebSocket + relative json/rest URLs), and relays the binary KVM stream.
//!
//! For Dell iDRAC machines (no JSON IRC), callers fall back to SOL (see
//! `crate::integration::bmc::ipmi::IpmiClient::sol_activate`) plus an
//! "open iDRAC console in a new tab" link.

pub mod asset;
pub mod kvm;
pub mod session;
