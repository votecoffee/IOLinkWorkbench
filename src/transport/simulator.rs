use super::{Freshness, Input, InputTransport, Payload, Status, Validity};
use crate::profile::{Identity, Profile};
use anyhow::{Result, bail};
use std::time::Instant;

const SOURCE_PERIOD_US: u64 = 1000;

pub struct Simulator {
    port: u8,
    width: usize,
    faults: bool,
    snapshots: bool,
    epoch: Instant,
    polls: u64,
    frozen_at: u64,
}

impl Simulator {
    pub fn new(profile: &Profile, faults: bool) -> Self {
        Self {
            port: profile.connection.port,
            width: profile.input_bytes,
            faults,
            snapshots: profile.connection.simulate_snapshots,
            epoch: Instant::now(),
            polls: 0,
            frozen_at: 0,
        }
    }
}

impl InputTransport for Simulator {
    fn connect(&mut self) -> Result<Status> {
        self.epoch = Instant::now();
        Ok(Status {
            route: "synthetic simulator",
            model: "Simulated IO-Link master".into(),
            firmware: "synthetic-1".into(),
            master_serial: None,
            port: self.port,
            port_mode: "IO-Link".into(),
            device_status: Some("Operate".into()),
            port_event: None,
            device_event: None,
            port_cycle_raw: Some("1000 us (simulated)".into()),
            port_cycle_us: Some(SOURCE_PERIOD_US),
            identity: Identity {
                vendor_id: Some(65535),
                device_id: Some(1),
                product_id: Some("SIM-SENSOR".into()),
                product_name: Some("Simulated level sensor".into()),
                serial: Some("SIM-0001".into()),
            },
            freshness: if self.snapshots {
                Freshness::Snapshots
            } else {
                Freshness::Counter32
            },
        })
    }

    fn poll(&mut self) -> Result<Input> {
        self.polls += 1;
        if self.faults && self.polls == 91 {
            bail!("simulated connection loss");
        }
        let received = Instant::now();
        let now = received.duration_since(self.epoch).as_micros() as u64;
        let mut source_time = now / SOURCE_PERIOD_US * SOURCE_PERIOD_US;
        if self.faults {
            match self.polls {
                20 => self.frozen_at = source_time,
                21..=39 => source_time = self.frozen_at,
                _ => {}
            }
        }
        let count = source_time / SOURCE_PERIOD_US;
        let level = (count / 5 % 200) as i16 - 100;
        let value = ((level << 1) | ((count / 25 % 2) as i16)).to_be_bytes();
        let mut raw = [0; crate::decode::MAX_INPUT];
        if self.width == 1 {
            raw[0] = value[1];
        } else {
            raw[self.width - 2..self.width].copy_from_slice(&value);
        }
        let invalid = self.faults && (55..=60).contains(&self.polls);
        Ok(Input {
            received,
            payload: Payload::new(&raw[..self.width])?,
            validity: if invalid {
                Validity::Invalid
            } else {
                Validity::Valid
            },
            counter: (!self.snapshots).then_some(count as u32),
            source_time_us: (!self.snapshots).then_some(source_time),
            source_age_us: (!self.snapshots).then_some(now - source_time),
            diagnostic_code: invalid.then_some(530),
        })
    }
}

pub fn read_isdu(index: u16, subindex: u8) -> Result<Vec<u8>> {
    match (index, subindex) {
        (18, 0) => Ok(b"Simulated level sensor".to_vec()),
        (19, 0) => Ok(b"SIM-SENSOR".to_vec()),
        (21, 0) => Ok(b"SIM-0001".to_vec()),
        (36, 0) => Ok(vec![2]),
        (37, 0) => Ok(vec![0xE4, 0x8D, 0xFE, 0, 0, 0]),
        (64, 0) => Ok(500i16.to_be_bytes().to_vec()),
        (65, 0) => Ok(b"synthetic sensor".to_vec()),
        (66, 0) => Ok(vec![0xFF, 0xEC, 1, 10, b'T', b'E', b'S', b'T']),
        (66, 1) => Ok((-20i16).to_be_bytes().to_vec()),
        (66, 2) => Ok(vec![1]),
        (66, 3) => Ok(vec![10]),
        (66, 4) => Ok(b"TEST".to_vec()),
        (67, 0) => Ok(vec![0x02, 0xC4, 0x95, 0xF0]),
        (67, 1..=7) => Ok(vec![
            [2i8, -4, 4, -7, 5, -1, 0][usize::from(subindex - 1)] as u8,
        ]),
        (71, 0) | (73, 1) | (74, 1) => Ok(0x9DFF_4400_8000_0000u64.to_be_bytes().to_vec()),
        (72, 0) | (73, 2) => Ok(0xFFFF_FFFD_4000_0000u64.to_be_bytes().to_vec()),
        (73, 0) => Ok([
            0x9DFF_4400_8000_0000u64.to_be_bytes(),
            0xFFFF_FFFD_4000_0000u64.to_be_bytes(),
        ]
        .concat()),
        (74, 0) => Ok([0x9DFF_4400_8000_0000u64.to_be_bytes(), 0u64.to_be_bytes()].concat()),
        (74, 2) => Ok(0u64.to_be_bytes().to_vec()),
        (75, 0) => Ok([
            0x0000_0001_8000_0000u64.to_be_bytes(),
            i64::MIN.to_be_bytes(),
        ]
        .concat()),
        (75, 1) => Ok(0x0000_0001_8000_0000u64.to_be_bytes().to_vec()),
        (75, 2) => Ok(i64::MIN.to_be_bytes().to_vec()),
        (76, 0) | (78, 1) | (79, 1) => Ok(vec![0, 0x80, 0xFF, 0x2C]),
        (77, 0) | (78, 2) => Ok(vec![0xC2, 0xB5, b'm', 0, 0, 0, 0, 0]),
        (78, 0) => Ok(vec![
            0, 0x80, 0xFF, 0x2C, 0xC2, 0xB5, b'm', 0, 0, 0, 0, 0, b'O', b'K', 0, 0,
        ]),
        (78, 3) | (80, 2) => Ok(vec![b'O', b'K', 0, 0]),
        (79, 0) => Ok(vec![0, 0x80, 0xFF, 0x2C, 0x10, 0x20, 0, 0]),
        (79, 2) => Ok(vec![0x10, 0x20, 0, 0]),
        (80, 0) => Ok(vec![0xC3, 0xA9, 0, 0, b'O', b'K', 0, 0]),
        (80, 1) => Ok(vec![0xC3, 0xA9]),
        (48, 0) => Ok(vec![42]),
        (81, 0) | (84, 1) => Ok(vec![1]),
        (82, 0) => Ok(u64::MAX.to_be_bytes().to_vec()),
        (83, 0) => Ok(1.25f32.to_be_bytes().to_vec()),
        (84, 0) => Ok(vec![1, 0xFF, 0xFB, 3]),
        (84, 2) => Ok((-5i16).to_be_bytes().to_vec()),
        (84, 3) | (85, 3) => Ok(vec![3]),
        (85, 0) => Ok(vec![0x0F, 0x23]),
        (85, 1) => Ok(vec![0xFF]),
        (85, 2) => Ok(vec![2]),
        _ => {
            bail!(
                "simulator defines identity (18:0, 19:0, 21:0), diagnostics (36:0, 37:0), synthetic offset (48:0), threshold (64:0), label (65:0), record (66:0 through 66:4), array (67:0 through 67:7), time parameters (71:0 through 75:2), text/octet parameters (76:0 through 80:2; record 78 also has subindex 3), and value-definition parameters (81:0 through 83:0, 84:0 through 85:3)"
            )
        }
    }
}
