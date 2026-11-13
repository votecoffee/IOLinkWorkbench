//! Explicit little-endian wire offsets; never reinterpret Rust struct padding as ABI.
use anyhow::{Result, ensure};
use serde::Serialize;

pub const MAP_BYTES: usize = 1_048_576;
pub const HEADER_BYTES: usize = 64;
pub const MAGIC: &[u8; 8] = b"IOLWSHM\0";

#[derive(Debug, Serialize)]
pub struct Header {
    pub abi_version: u32,
    pub payload_bytes: u32,
    pub publish_sequence: u64,
    pub publisher_pid: u32,
    pub flags: u32,
    pub published_qpc_ticks: u64,
    pub qpc_frequency: u64,
    pub process_creation_time: u64,
}
impl Header {
    pub fn bytes(&self) -> [u8; HEADER_BYTES] {
        let mut data = [0; HEADER_BYTES];
        data[..8].copy_from_slice(MAGIC);
        for (offset, value) in [
            (8, self.abi_version),
            (12, HEADER_BYTES as u32),
            (16, MAP_BYTES as u32),
            (20, self.payload_bytes),
            (32, self.publisher_pid),
            (36, self.flags),
        ] {
            data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        for (offset, value) in [
            (24, self.publish_sequence),
            (40, self.published_qpc_ticks),
            (48, self.qpc_frequency),
            (56, self.process_creation_time),
        ] {
            data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
        }
        data
    }
    pub fn parse(data: &[u8]) -> Result<Self> {
        ensure!(
            data.len() == HEADER_BYTES && &data[..8] == MAGIC,
            "uninitialized or invalid shared-memory header"
        );
        let u32_at = |offset| {
            u32::from_le_bytes(data[offset..offset + 4].try_into().expect("bounded offset"))
        };
        let u64_at = |offset| {
            u64::from_le_bytes(data[offset..offset + 8].try_into().expect("bounded offset"))
        };
        ensure!(
            u32_at(8) == 1 && u32_at(12) == HEADER_BYTES as u32 && u32_at(16) == MAP_BYTES as u32,
            "unsupported shared-memory layout"
        );
        let header = Self {
            abi_version: 1,
            payload_bytes: u32_at(20),
            publish_sequence: u64_at(24),
            publisher_pid: u32_at(32),
            flags: u32_at(36),
            published_qpc_ticks: u64_at(40),
            qpc_frequency: u64_at(48),
            process_creation_time: u64_at(56),
        };
        ensure!(
            (1..=MAP_BYTES - HEADER_BYTES).contains(&(header.payload_bytes as usize))
                && header.publish_sequence > 0
                && header.publisher_pid > 0
                && header.flags & !3 == 0
                && header.qpc_frequency > 0
                && header.process_creation_time > 0,
            "invalid shared-memory bounds/metadata"
        );
        Ok(header)
    }
}
