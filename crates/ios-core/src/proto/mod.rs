//! Protocol type definitions and codecs for iOS device communication.
//!
//! This crate contains zero-IO, zero-async protocol types shared by all upper-layer crates:
//! - **AFC** — Apple File Conduit binary protocol
//! - **DTX** — DTXMessage RPC framing (Instruments, TestManager)
//! - **Lockdown** — Lockdown plist request/response types
//! - **NSKeyedArchiver** — NSKeyedArchiver/NSKeyedUnarchiver encode/decode
//! - **OPACK** — OPACK binary serialization (XPC, RemoteXPC)
//! - **XPC** — XPC message types

#[allow(dead_code)]
pub mod afc;
#[allow(dead_code)]
pub mod dtx;
#[allow(dead_code)]
pub mod lockdown;
#[allow(dead_code)]
pub mod nskeyedarchiver;
pub mod nskeyedarchiver_encode;
#[allow(dead_code)]
pub mod opack;
pub mod tls;
#[cfg(feature = "tunnel")]
pub mod tlv;
#[allow(dead_code)]
pub mod usbmuxd;
#[allow(dead_code)]
pub mod xpc;
