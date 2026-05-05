// ============================================================================
// ULTRACLAW — Connectors Module
// ============================================================================
// Exports platform-specific connector implementations.

#[cfg(feature = "discord")]
pub mod discord;

#[cfg(feature = "matrix")]
pub mod matrix;

#[cfg(feature = "telegram")]
pub mod telegram;

#[cfg(feature = "webhook")]
pub mod webhook;
pub mod massive_channels;

pub mod mobile;
pub mod firmware;

pub mod android_client;
pub mod swabble_mac;
pub mod phone_control;