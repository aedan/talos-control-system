//! BMC HTML5 remote consoles (in-portal).
//!
//! Lets an operator open a machine's out-of-band console inside TCS without the
//! browser needing to reach the BMC directly (management subnet, self-signed
//! cert, `X-Frame-Options: sameorigin`).
//!
//! * HPE iLO: JSON login + proxied `irc.html` + DVCNET KVM WebSocket.
//! * Dell iDRAC: Redfish `GetKVMSession` (or legacy GUI login) + reverse-proxied
//!   HTML5 / eHTML5 virtual console + WebSocket relay. No SOL fallback.

pub mod asset;
pub mod idrac;
pub mod idrac_proxy;
pub mod kvm;
pub mod session;
pub mod tls;
