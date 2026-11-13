use super::*;
use std::{
    io::{Read, Write},
    net::TcpStream,
};

const IO_BUDGET: Duration = Duration::from_millis(250);
const MAX_HEADERS: usize = 4096;

pub(super) struct Server {
    pub shared: Arc<Shared>,
    pub store: Arc<Mutex<Store>>,
    pub schema: Value,
    pub profile: Profile,
    pub session_id: String,
    pub utc_start: String,
    pub start: Instant,
    pub authority: String,
    pub queue_capacity: usize,
    pub history_capacity: usize,
}

impl Server {
    pub(super) fn run(self, listener: TcpListener) -> Result<()> {
        while !self.shared.stop.load(Ordering::Acquire) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    self.shared.requests.fetch_add(1, Ordering::Relaxed);
                    if self.handle(&mut stream).is_err() {
                        self.shared.failed_requests.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(2))
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }

    fn handle(&self, stream: &mut TcpStream) -> Result<()> {
        // Accepted sockets can inherit nonblocking mode on Windows.
        stream.set_nonblocking(false)?;
        let (code, value) = match read_headers(stream) {
            Ok(headers) => self.route(&headers),
            Err(_) => (
                400,
                json!({"error":"incomplete, oversized or timed-out headers"}),
            ),
        };
        let mut bytes = serde_json::to_vec(&value)?;
        let code = if bytes.len() > MAX_REPLY {
            bytes = br#"{"error":"response exceeds 1 MiB limit"}"#.to_vec();
            413
        } else {
            code
        };
        let label = match code {
            200 => "OK",
            400 => "Bad Request",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            409 => "Conflict",
            413 => "Content Too Large",
            _ => "Error",
        };
        let header = format!(
            "HTTP/1.1 {code} {label}\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n",
            bytes.len()
        );
        let deadline = Instant::now() + IO_BUDGET;
        write_before(stream, header.as_bytes(), deadline)?;
        write_before(stream, &bytes, deadline)?;
        ensure!(code == 200, "rejected local API request");
        Ok(())
    }

    pub(super) fn route(&self, headers: &str) -> (u16, Value) {
        match self.request(headers) {
            Ok(reply) => reply,
            Err(error) => (400, json!({"error":error.to_string()})),
        }
    }

    fn request(&self, headers: &str) -> Result<(u16, Value)> {
        let mut lines = headers.split("\r\n");
        let mut request = lines.next().unwrap_or("").split(' ');
        let method = request.next().unwrap_or("");
        let target = request.next().unwrap_or("");
        ensure!(
            request.next() == Some("HTTP/1.1") && request.next().is_none(),
            "expected HTTP/1.1 origin-form request"
        );
        let mut host = None;
        for line in lines.filter(|line| !line.is_empty()) {
            let (name, value) = line.split_once(':').context("malformed header")?;
            ensure!(
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "malformed header name"
            );
            let value = value.trim();
            if name.eq_ignore_ascii_case("host") {
                ensure!(host.is_none(), "duplicate Host");
                host = Some(value);
            }
            if name.eq_ignore_ascii_case("origin") {
                return Ok((
                    403,
                    json!({"error":"browser Origin requests are not supported"}),
                ));
            }
            ensure!(
                !name.eq_ignore_ascii_case("transfer-encoding")
                    && (!name.eq_ignore_ascii_case("content-length") || value == "0"),
                "request bodies are not supported"
            );
        }
        if host != Some(self.authority.as_str()) {
            return Ok((
                403,
                json!({"error":"Host must match the bound numeric loopback authority"}),
            ));
        }
        if method != "GET" {
            return Ok((405, json!({"error":"read-only API supports GET only"})));
        }
        ensure!(
            target.starts_with('/') && !target.starts_with("//") && !target.contains('#'),
            "expected origin-form target"
        );
        let (path, query) = target.split_once('?').unwrap_or((target, ""));
        match path {
            "/v1/schema" => {
                ensure!(query.is_empty(), "schema accepts no query");
                Ok((200, self.envelope(json!({"schema":self.schema}))))
            }
            "/v1/status" => {
                ensure!(query.is_empty(), "status accepts no query");
                let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
                let latest = store.history.back();
                let master = store.master.as_ref().filter(|(generation, _, _)| {
                    latest.is_some_and(|e| e.generation == *generation)
                });
                Ok((200,self.envelope(json!({"metrics":self.metrics(&store),"latest_event":latest.map(|e|event_json(e,&self.profile)),
                    "latest_event_age_us":latest.map(|e|micros(self.start.elapsed()).saturating_sub(e.elapsed_us)),
                    "observed_master":master.map(|(_,s,_)|s),"master_text_limit_utf8_bytes":256,"master_text_truncated":master.map(|(_,_,t)|t),
                    "status_basis":"last ingested events; queue lag/drops and event age must be checked; not a fresh gateway query"}))))
            }
            "/v1/history" => self.history(query),
            _ => Ok((404, json!({"error":"unknown read-only endpoint"}))),
        }
    }

    fn history(&self, query: &str) -> Result<(u16, Value)> {
        let mut after = None;
        let mut session = None;
        let mut limit = None;
        if !query.is_empty() {
            for pair in query.split('&') {
                let (name, value) = pair.split_once('=').context("expected key=value query")?;
                match name {
                    "after" => {
                        ensure!(
                            after.is_none()
                                && !value.is_empty()
                                && value.bytes().all(|b| b.is_ascii_digit()),
                            "invalid/duplicate after"
                        );
                        after = Some(value.parse::<u64>()?);
                    }
                    "session" => {
                        ensure!(session.is_none(), "duplicate session");
                        session = Some(value);
                    }
                    "limit" => {
                        ensure!(
                            limit.is_none()
                                && !value.is_empty()
                                && value.bytes().all(|b| b.is_ascii_digit()),
                            "invalid/duplicate limit"
                        );
                        limit = Some(value.parse::<usize>()?);
                    }
                    _ => anyhow::bail!("unknown query key"),
                }
            }
        }
        ensure!(
            after.is_some() == session.is_some(),
            "after and session must be supplied together"
        );
        if session.is_some_and(|s| s != self.session_id) {
            return Ok((
                409,
                json!({"error":"session changed; restart cursor","session_id":self.session_id}),
            ));
        }
        let after = after.unwrap_or(0);
        let limit = limit.unwrap_or(64);
        ensure!((1..=MAX_LIMIT).contains(&limit), "limit must be 1–128");
        ensure!(
            after <= self.shared.offered.load(Ordering::Relaxed),
            "cursor is ahead of offered events"
        );
        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        let oldest = store.history.front().map(|e| e.sequence);
        let mut events = Vec::new();
        let mut size = 0;
        let mut next = after;
        let mut size_limited = false;
        for event in store
            .history
            .iter()
            .filter(|e| e.sequence > after)
            .take(limit)
        {
            let value = event_json(event, &self.profile);
            size += serde_json::to_vec(&value)?.len() + 1;
            if size > MAX_REPLY - 16384 {
                size_limited = true;
                break;
            }
            next = event.sequence;
            events.push(value);
        }
        Ok((200,self.envelope(json!({"metrics":self.metrics(&store),"events":events,"next_after":next,
            "missed_before_oldest":oldest.map(|first|first.saturating_sub(1).saturating_sub(after)).unwrap_or(0),
            "more_available":store.history.back().is_some_and(|e|e.sequence>next),"size_limited":size_limited}))))
    }

    fn envelope(&self, body: Value) -> Value {
        json!({"format_version":1,"session_id":self.session_id,"capture_utc_start":self.utc_start,"host_elapsed_us":micros(self.start.elapsed()),"body":body})
    }
    fn metrics(&self, store: &Store) -> Value {
        json!({"events_offered":self.shared.offered.load(Ordering::Relaxed),"queue_dropped":self.shared.dropped.load(Ordering::Relaxed),"events_ingested":store.ingested,"history_evicted":store.evicted,
        "queue_capacity":self.queue_capacity,"history_capacity":self.history_capacity,"oldest_sequence":store.history.front().map(|e|e.sequence),"newest_sequence":store.history.back().map(|e|e.sequence),
        "requests":self.shared.requests.load(Ordering::Relaxed),"failed_requests":self.shared.failed_requests.load(Ordering::Relaxed)})
    }
}

fn read_headers(stream: &mut TcpStream) -> Result<String> {
    let deadline = Instant::now() + IO_BUDGET;
    let mut headers = Vec::with_capacity(MAX_HEADERS);
    let mut chunk = [0; 512];
    loop {
        stream.set_read_timeout(Some(remaining(deadline)?))?;
        let size = stream.read(&mut chunk)?;
        ensure!(
            size > 0 && headers.len() + size <= MAX_HEADERS,
            "incomplete or oversized request"
        );
        headers.extend_from_slice(&chunk[..size]);
        if let Some(end) = headers.windows(4).position(|b| b == b"\r\n\r\n") {
            ensure!(
                end + 4 == headers.len(),
                "bodies/pipelining are not supported"
            );
            return Ok(std::str::from_utf8(&headers)?.to_owned());
        }
    }
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .context("request deadline exceeded")
}
fn write_before(stream: &mut TcpStream, mut bytes: &[u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        stream.set_write_timeout(Some(remaining(deadline)?))?;
        let count = stream.write(bytes)?;
        ensure!(count > 0, "client closed response");
        bytes = &bytes[count..];
    }
    Ok(())
}
