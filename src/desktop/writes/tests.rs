use super::*;
use crate::desktop::{Tab, WORKER_TEST_LOCK, setup, shutdown::CloseAction};
use anyhow::bail;
use eframe::egui::Context as EguiContext;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

fn fixture_profile(endpoint: String) -> Result<Profile> {
    let mut profile = Profile::parse(include_str!("../../../samples/simulator.toml"))?;
    profile.connection.gateway = Gateway::IfmAl1350;
    profile.connection.endpoint = endpoint;
    profile.connection.ifm_firmware = "3.1.42".into();
    profile.connection.ifm_mode = Some(3);
    profile.expected_identity.product_id = None;
    Ok(profile)
}

#[test]
fn draft_forms_stage_the_shared_wire_preview_offline() -> Result<()> {
    let profile = fixture_profile("https://127.0.0.1:9".into())?;
    let mut panel = Panel::default();
    for (draft, payload) in [
        (
            Draft {
                expected: "3".into(),
                desired: "0".into(),
                ..Default::default()
            },
            json!({"newvalue":0}),
        ),
        (
            Draft {
                kind: Kind::Parameter,
                index: 65,
                subindex: 1,
                expected: "00".into(),
                desired: "01".into(),
                ..Default::default()
            },
            json!({"index":65,"subindex":1,"value":"01"}),
        ),
        (
            Draft {
                kind: Kind::Command,
                code: 160,
                meaning: "Synthetic test command".into(),
                ..Default::default()
            },
            json!({"index":2,"subindex":0,"value":"A0"}),
        ),
        (
            Draft {
                kind: Kind::Output,
                expected: "00 00".into(),
                desired: "00 01".into(),
                ..Default::default()
            },
            json!({"newvalue":"0001"}),
        ),
    ] {
        panel.approved = true;
        panel.stage(Plan::new(
            &profile,
            "BENCH-MASTER",
            draft.operation(&profile)?,
        )?);
        assert!(!panel.approved && !panel.busy());
        let review = panel.review.as_ref().unwrap().review();
        assert_eq!(review.request_data, payload);
        assert_eq!(review.master_serial, "BENCH-MASTER");
    }
    panel.approved = true;
    panel.clear_draft();
    assert!(panel.review.is_none() && !panel.approved);
    Ok(())
}

#[test]
fn typed_isdu_draft_stages_exact_bytes_and_keeps_expected_hex() -> Result<()> {
    let mut profile = fixture_profile("https://127.0.0.1:9".into())?;
    let catalog = iolink_workbench::iodd::parse(
        include_str!("../../../samples/sensor-formats.xml"),
        "synthetic write form".into(),
    )?;
    let (_, imported) = catalog.select(None, None)?;
    profile.input_bytes = imported.input_bytes;
    profile.iodd = Some(imported);
    let mut draft = Draft {
        kind: Kind::Parameter,
        index: 64,
        expected: "0001".into(),
        desired: "-2".into(),
        typed_desired: true,
        ..Default::default()
    };
    let plan = Plan::new(&profile, "SYNTHETIC-MASTER", draft.operation(&profile)?)?;
    let review = plan.review();
    assert_eq!(review.expected_value, Some(json!("0001")));
    assert_eq!(review.request_data["value"], "FFFE");
    draft.index = 65;
    assert!(draft.operation(&profile).is_err());
    draft.reset_values();
    assert!(draft.expected.is_empty() && draft.desired.is_empty() && !draft.typed_desired);
    Ok(())
}

fn read_request(listener: &TcpListener) -> Result<(TcpStream, Value)> {
    let deadline = Instant::now() + Duration::from_secs(5);
    let stream = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(error).context("wait for fixture request"),
        }
    };
    stream.set_nonblocking(false)?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    let mut reader = BufReader::new(&stream);
    let mut len = None;
    loop {
        let mut header = String::new();
        ensure!(
            reader.read_line(&mut header)? > 0,
            "fixture request ended in headers"
        );
        if header == "\r\n" {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            len = Some(value.trim().parse::<usize>()?);
        }
    }
    let mut payload = vec![0; len.context("fixture request missing Content-Length")?];
    reader.read_exact(&mut payload)?;
    Ok((stream, serde_json::from_slice(&payload)?))
}

fn reply_with_identity(listener: &TcpListener) -> Result<()> {
    let (mut stream, preflight) = read_request(listener)?;
    ensure!(
        preflight["adr"] == "/getdatamulti",
        "expected identity preflight"
    );
    let body = json!({"cid":preflight["cid"], "code":200, "data":{
        "/deviceinfo/productcode":{"code":200,"data":"AL1350"},
        "/deviceinfo/swrevision":{"code":200,"data":"3.1.42"},
        "/deviceinfo/serialnumber":{"code":200,"data":"BENCH-MASTER"},
        "/iolinkmaster/port[1]/mode":{"code":200,"data":3},
        "/iolinkmaster/port[1]/iolinkdevice/vendorid":{"code":200,"data":65535},
        "/iolinkmaster/port[1]/iolinkdevice/deviceid":{"code":200,"data":1},
    }})
    .to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    Ok(())
}

#[test]
fn current_value_uses_an_independent_read_and_stays_bound_to_the_draft() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut app = App::new();
    app.profile = fixture_profile(format!("http://{}", listener.local_addr()?))?;
    app.setup.sync(&app.profile);
    app.writes.master_serial = "BENCH-MASTER".into();
    app.writes.draft.expected = "1".into();
    let (_hold_capture, completion) = mpsc::channel();
    app.worker = Some(completion);
    assert!(app.write_block_reason().is_some());
    assert!(app.current_read_block_reason().is_none());

    let peer = thread::spawn(move || reply_with_identity(&listener));
    app.start_current_read()?;
    assert_eq!(app.pending_jobs(), 2);
    assert!(!app.writes.writing());
    peer.join().unwrap()?;
    let ctx = EguiContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.writes.busy() && Instant::now() < deadline {
        let _ = ctx.run(Default::default(), |ctx| app.render(ctx));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.pending_jobs(), 1);
    assert_eq!(app.writes.draft.expected, "1");
    assert_eq!(app.writes.current_for_draft().unwrap().value_text(), "3");
    assert!(app.writes.review.is_none() && !app.writes.approved);
    app.writes.master_serial = "DIFFERENT-MASTER".into();
    assert!(app.writes.current_for_draft().is_none());
    app.writes.master_serial = "BENCH-MASTER".into();
    app.writes.draft.kind = Kind::Output;
    assert!(app.writes.current_for_draft().is_none());
    app.writes.clear_draft();
    assert!(app.writes.current.is_none());
    Ok(())
}

#[test]
fn reviewed_gui_write_waits_on_close_and_retains_a_lost_reply() -> Result<()> {
    let _serial = WORKER_TEST_LOCK.lock().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let mut app = App::new();
    app.profile = fixture_profile(format!("http://{}", listener.local_addr()?))?;
    app.setup.sync(&app.profile);
    app.writes.stage(Plan::new(
        &app.profile,
        "BENCH-MASTER",
        Operation::DeviceCommand {
            code: 160,
            meaning: "Synthetic test command".into(),
        },
    )?);
    assert!(
        app.start_reviewed_write()
            .unwrap_err()
            .to_string()
            .contains("Approve")
    );
    app.writes.approved = true;
    let (active_read, read_reply) = mpsc::channel();
    app.acyclic.hold_for_test(read_reply);
    assert!(
        app.start_reviewed_write()
            .unwrap_err()
            .to_string()
            .contains("active parameter read")
    );
    drop(active_read);
    app.acyclic.update();

    let (observed, request_sent) = mpsc::channel();
    let (release, release_reply) = mpsc::channel();
    let peer = thread::spawn(move || -> Result<()> {
        reply_with_identity(&listener)?;
        let (stream, write) = read_request(&listener)?;
        ensure!(write["adr"] == "/iolinkmaster/port[1]/iolinkdevice/iolwriteacyclic");
        ensure!(write["data"] == json!({"index":2,"subindex":0,"value":"A0"}));
        observed.send(())?;
        release_reply.recv_timeout(Duration::from_secs(5))?;
        drop(stream);
        // Allow a retry to show up as another connection to the isolated peer.
        thread::sleep(Duration::from_millis(60));
        match listener.accept() {
            Err(error) if error.kind() == ErrorKind::WouldBlock => Ok(()),
            _ => bail!("the write was retried"),
        }
    });

    app.start_reviewed_write()?;
    request_sent.recv_timeout(Duration::from_secs(5))?;
    assert!(app.writes.busy() && app.writes.review.is_none() && !app.writes.approved);
    assert!(app.start_block_reason().unwrap().contains("write"));
    assert!(app.acyclic_block_reason().unwrap().contains("write"));
    assert_eq!(app.pending_jobs(), 1);
    app.tab = Tab::Inputs;
    assert_eq!(app.advance_close(true), CloseAction::Wait);
    release.send(())?;
    let ctx = EguiContext::default();
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.pending_jobs() != 0 && Instant::now() < deadline {
        let _ = ctx.run(Default::default(), |ctx| app.render(ctx));
        thread::sleep(Duration::from_millis(5));
    }
    peer.join().unwrap()?;
    assert_eq!(app.pending_jobs(), 0);
    assert!(!app.closing && app.message.contains("Close canceled"));
    assert_eq!(app.tab, Tab::Device);
    assert_eq!(app.setup.page, setup::Page::Writes);
    let result = app.writes.result.as_ref().unwrap();
    assert!(result.unconfirmed);
    let receipt: Value = serde_json::from_str(result.receipt.as_deref().unwrap())?;
    assert_eq!(receipt["outcome"], "unconfirmed");
    assert_eq!(
        receipt["preview"]["target_and_request"]["master_serial"],
        "BENCH-MASTER"
    );
    Ok(())
}
