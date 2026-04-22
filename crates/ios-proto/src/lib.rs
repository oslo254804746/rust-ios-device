//! Protocol type definitions and codecs for iOS device communication.
//!
//! This crate contains zero-IO, zero-async protocol types shared by all upper-layer crates:
//! - **AFC** — Apple File Conduit binary protocol
//! - **DTX** — DTXMessage RPC framing (Instruments, TestManager)
//! - **Lockdown** — Lockdown plist request/response types
//! - **NSKeyedArchiver** — NSKeyedArchiver/NSKeyedUnarchiver encode/decode
//! - **OPACK** — OPACK binary serialization (XPC, RemoteXPC)
//! - **XPC** — XPC message types

pub mod afc;
pub mod dtx;
pub mod lockdown;
pub mod nskeyedarchiver;
pub mod nskeyedarchiver_encode;
pub mod opack;
pub mod tls;
pub mod tlv;
pub mod usbmuxd;
pub mod xpc;
