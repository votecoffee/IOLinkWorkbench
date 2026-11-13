//! Publisher-only MQTT 3.1.1 framing. No subscriptions, command handling or QoS retries.
use anyhow::{Context, Result, ensure};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

pub(super) const IO_BUDGET: Duration = Duration::from_millis(250);
const KEEP_ALIVE: u16 = 10;
const PING_AFTER: Duration = Duration::from_secs(5);
const PING_TIMEOUT: Duration = Duration::from_secs(1);
pub(super) const MAX_PAYLOAD: usize = 1_048_576;

fn packet(header: u8, body: &[u8]) -> Vec<u8> {
    let mut result = vec![header];
    let mut length = body.len();
    loop {
        let mut byte = (length % 128) as u8;
        length /= 128;
        if length != 0 {
            byte |= 128;
        }
        result.push(byte);
        if length == 0 {
            break;
        }
    }
    result.extend_from_slice(body);
    result
}

pub(super) fn connect_packet(client_id: &str) -> Vec<u8> {
    let mut body = b"\0\x04MQTT\x04\x02".to_vec();
    body.extend_from_slice(&KEEP_ALIVE.to_be_bytes());
    body.extend_from_slice(&(client_id.len() as u16).to_be_bytes());
    body.extend_from_slice(client_id.as_bytes());
    packet(0x10, &body)
}
pub(super) fn publish_packet(topic: &str, payload: &[u8]) -> Result<Vec<u8>> {
    ensure!(
        payload.len() <= MAX_PAYLOAD && topic.len() <= 512,
        "MQTT message exceeds bounds"
    );
    let mut body = Vec::with_capacity(2 + topic.len() + payload.len());
    body.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    body.extend_from_slice(topic.as_bytes());
    body.extend_from_slice(payload);
    Ok(packet(0x30, &body))
}

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .context("MQTT I/O deadline exceeded")
}
fn write_deadline(stream: &mut TcpStream, mut bytes: &[u8]) -> Result<()> {
    let deadline = Instant::now() + IO_BUDGET;
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        let count = stream.write(bytes)?;
        ensure!(count > 0, "MQTT socket closed while writing");
        bytes = &bytes[count..];
    }
    Ok(())
}

pub(super) struct Connection {
    stream: TcpStream,
    last_send: Instant,
    ping: Option<Instant>,
    reply: [u8; 2],
    used: usize,
}
impl Connection {
    pub(super) fn connect(address: SocketAddr, client_id: &str) -> Result<Self> {
        let mut stream = TcpStream::connect_timeout(&address, IO_BUDGET)?;
        stream.set_nodelay(true)?;
        write_deadline(&mut stream, &connect_packet(client_id))?;
        let deadline = Instant::now() + IO_BUDGET;
        let mut response = [0; 4];
        let mut used = 0;
        while used < response.len() {
            stream.set_read_timeout(Some(remaining(deadline)?))?;
            let count = stream.read(&mut response[used..])?;
            ensure!(count > 0, "broker closed before CONNACK");
            used += count;
        }
        ensure!(
            response[0..3] == [0x20, 2, 0],
            "invalid CONNACK or unexpected session-present flag"
        );
        ensure!(
            response[3] == 0,
            "broker rejected CONNECT (code {})",
            response[3]
        );
        Ok(Self {
            stream,
            last_send: Instant::now(),
            ping: None,
            reply: [0; 2],
            used: 0,
        })
    }
    pub(super) fn publish(&mut self, topic: &str, payload: &[u8]) -> Result<()> {
        self.stream.set_nonblocking(false)?;
        write_deadline(&mut self.stream, &publish_packet(topic, payload)?)?;
        self.last_send = Instant::now();
        Ok(())
    }
    /// Returns true only when a PINGREQ was written on this turn.
    pub(super) fn maintain(&mut self) -> Result<bool> {
        self.stream.set_nonblocking(true)?;
        for _ in 0..4 {
            match self.stream.read(&mut self.reply[self.used..]) {
                Ok(0) => anyhow::bail!("MQTT connection closed"),
                Ok(count) => {
                    self.used += count;
                    if self.used == 2 {
                        ensure!(
                            self.reply == [0xD0, 0] && self.ping.take().is_some(),
                            "unexpected/malformed broker packet"
                        );
                        self.used = 0;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error.into()),
            }
        }
        ensure!(
            self.ping.is_none_or(|sent| sent.elapsed() < PING_TIMEOUT),
            "MQTT PINGRESP timeout"
        );
        if self.ping.is_none() && self.last_send.elapsed() >= PING_AFTER {
            self.stream.set_nonblocking(false)?;
            write_deadline(&mut self.stream, &[0xC0, 0])?;
            self.last_send = Instant::now();
            self.ping = Some(self.last_send);
            return Ok(true);
        }
        Ok(false)
    }
    pub(super) fn disconnect(mut self) -> bool {
        self.stream.set_nonblocking(false).is_ok()
            && write_deadline(&mut self.stream, &[0xE0, 0]).is_ok()
    }
}

#[cfg(test)]
pub(super) fn expire_keepalive(connection: &mut Connection) {
    connection.last_send = Instant::now() - PING_AFTER;
}
#[cfg(test)]
pub(super) fn expire_ping(connection: &mut Connection) {
    connection.ping = Some(Instant::now() - PING_TIMEOUT);
}
