use super::super::{
    App, Tab, configure_context,
    observation::{Observation, State},
};
use anyhow::{Context as _, Result, ensure};
use eframe::egui::{self, Context};
use iolink_workbench::{
    capture::{Live, Report},
    iodd,
    profile::{Gateway, Profile},
    transport::{Freshness, Input, Payload, Status, Validity},
};
use serde::Serialize;
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

pub(super) const SIZE: [u32; 2] = [1130, 790];
const MASTER_INSPECTION_FIXTURE: &str = concat!(
    "Master temperature 33 °C · Supply voltage 24050 mV · Sensor current 105 mA\n",
    "Supply status (raw) 0\n\n",
    "Port 8 · mode Disabled · Device identity unavailable",
);

#[derive(Serialize)]
pub(super) struct Case {
    pub profile: &'static str,
    pub tab: &'static str,
    pub width: u32,
    pub height: u32,
    pub passes: u8,
    pub shapes: usize,
    pub meshes: usize,
    pub triangles: usize,
}

fn profiles() -> Result<Vec<(&'static str, Profile)>> {
    let mut profiles = vec![(
        "bundled-simulator",
        Profile::parse(include_str!("../../../samples/simulator.toml"))?,
    )];
    for (name, xml) in [
        (
            "formats",
            include_str!("../../../samples/sensor-formats.xml"),
        ),
        (
            "records",
            include_str!("../../../samples/sensor-record.xml"),
        ),
        ("arrays", include_str!("../../../samples/sensor-array.xml")),
        (
            "strings",
            include_str!("../../../samples/sensor-strings.xml"),
        ),
        ("time", include_str!("../../../samples/sensor-time.xml")),
        ("values", include_str!("../../../samples/sensor-values.xml")),
        (
            "defaults",
            include_str!("../../../samples/sensor-defaults.xml"),
        ),
    ] {
        let catalog = iodd::parse(xml, format!("built-in synthetic GUI self-test {name}"))?;
        let (expected_identity, imported) = catalog.select(None, None)?;
        profiles.push((
            name,
            Profile {
                input_bytes: imported.input_bytes,
                expected_identity,
                iodd: Some(imported),
                ..Default::default()
            },
        ));
    }
    Ok(profiles)
}

fn observation_fixture(
    state: State,
    snapshots: bool,
) -> Result<(App, Option<mpsc::Sender<Result<Report>>>)> {
    let mut app = App::new();
    app.tab = Tab::Inputs;
    app.profile.connection.stale_us = 60_000_000;
    app.setup.sync(&app.profile);
    let now = Instant::now();
    let received = match state {
        State::HostSilence => now - Duration::from_secs(120),
        State::UpdateOld => now - Duration::from_secs(2),
        _ => now,
    };
    let last_new = if state == State::UpdateOld {
        // At receipt the update was 59 s old, under stale_us=60 s.
        now - Duration::from_secs(61)
    } else {
        received
    };
    app.live = Live {
        connected: true,
        generation: 1,
        status: Some(Status {
            route: "Synthetic GUI observation",
            model: "Synthetic GUI master".into(),
            firmware: "offline fixture".into(),
            master_serial: None,
            port: 1,
            port_mode: "IO-Link (modeled)".into(),
            device_status: None,
            port_event: None,
            device_event: None,
            port_cycle_raw: None,
            port_cycle_us: None,
            identity: app.profile.expected_identity.clone(),
            freshness: if snapshots {
                Freshness::Snapshots
            } else {
                Freshness::Counter32
            },
        }),
        input: Some(Input {
            received,
            payload: Payload::new(&[0x03, 0xe9])?,
            validity: if state == State::InvalidAtReceipt {
                Validity::Invalid
            } else {
                Validity::Valid
            },
            counter: (!snapshots).then_some(1),
            source_time_us: (!snapshots).then_some(1000),
            source_age_us: (!snapshots).then_some(0),
            diagnostic_code: None,
        }),
        last_new_at: (!snapshots).then_some(last_new),
        stale: (!snapshots).then_some(false),
        ..Default::default()
    };
    let gate = if state == State::Stopped {
        None
    } else {
        // A held completion channel models session state without dispatching a
        // thread, transport, listener or sampler. Keep its sender for both frames.
        let (sender, receiver) = mpsc::channel();
        app.worker = Some(receiver);
        Some(sender)
    };
    Ok((app, gate))
}

fn text_shapes<'a>(shape: &'a egui::Shape, texts: &mut Vec<&'a str>) {
    match shape {
        egui::Shape::Text(text) => texts.push(&text.galley.job.text),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                text_shapes(shape, texts);
            }
        }
        _ => {}
    }
}

fn apply_output_fixture(profile: &mut Profile) -> Result<()> {
    use iolink_workbench::decode::{Field, Kind};
    profile
        .iodd
        .as_mut()
        .context("missing IODD fixture")?
        .output = Some(iodd::ProcessDataLayout {
        id: "OUT".into(),
        bit_length: 13,
        packed: true,
        standalone_boolean: false,
        padded_integer: false,
        condition: None,
        fields: vec![Field {
            name: "Enable".into(),
            kind: Kind::Bool,
            bits: 1,
            ..Field::default()
        }],
        needs_input: Vec::new(),
    });
    Ok(())
}

pub(super) fn render(
    ctx: &Context,
    mut paint: impl FnMut([u32; 2], &egui::FullOutput, &[egui::ClippedPrimitive]) -> Result<()>,
) -> Result<Vec<Case>> {
    configure_context(ctx);
    // Exercise collapsed parameter/diagnostic/review controls without input events.
    ctx.memory_mut(|memory| memory.set_everything_is_visible(true));
    let mut views = Vec::new();
    for (name, profile) in profiles()? {
        ensure!(
            profile.connection.gateway == Gateway::Simulator,
            "GUI self-test profile must stay offline"
        );
        for tab in Tab::ALL {
            let mut app = App::new();
            app.profile = profile.clone();
            app.setup.sync(&app.profile);
            app.tab = tab;
            app.api_enabled = tab == Tab::Integration;
            app.mqtt_enabled = tab == Tab::Integration;
            app.shared_memory_enabled = tab == Tab::Integration;
            views.push((name, tab.label(), app, None, None));
        }
        if name == "formats" {
            for label in [
                "device-connection",
                "device-identity",
                "device-iodd",
                "writes-form",
                "writes-review",
                "writes-text-review",
                "writes-output-review",
                "writes-boolean-output",
                "device-advanced",
                "mapping-editor",
                "mapping-iodd-features",
                "layout-condition",
                "layout-condition-whole",
                "parameters-raw",
                "parameters-selected-record",
                "integration-mqtt",
                "integration-shared-memory",
                "diagnostics-timing",
                "master-inspection",
                "diagnostics-events",
                "diagnostics-device-profile",
                "offline-compare",
                "offline-signal",
                "offline-raw-signal",
                "offline-exports",
                "offline-process",
                "offline-process-output",
                "offline-padded-input",
                "offline-boolean-input",
                "offline-float-state-name",
                "offline-hex-format",
                "offline-named-range",
                "offline-octet-input",
                "offline-text-input",
                "offline-temporal-input",
                "acyclic-profile",
                "offline-parameter",
                "offline-profile-ids",
                "offline-process-descriptor",
                "offline-descriptor-plan",
                "offline-snapshot",
                "offline-events",
                "offline-master-snapshot",
                "offline-master-comparison",
                "offline-diagnostic-snapshot",
                "offline-diagnostic-comparison",
                "profiles-dialog",
                "profiles-files",
                "drop-preview",
                "drop-saved-preview",
                "latest-recording",
                "port-capture-review",
                "port-capture-results",
                "port-status-results",
                "connection-dialog",
                "connection-needs-endpoint",
                "al1351-connection",
                "al1352-connection",
                "al1353-connection",
                "connection-feed-settings",
            ] {
                let mut app = App::new();
                app.profile = profile.clone();
                app.setup.sync(&app.profile);
                use super::super::{
                    acyclic::{Output, Query},
                    setup::Page,
                    workspace::{
                        DiagnosticsPage, IntegrationPage, LoggingPage, OfflinePage, ParameterPage,
                    },
                };
                match label {
                    "device-connection" => app.setup.page = Page::Connection,
                    "device-identity" => app.setup.page = Page::Identity,
                    "device-iodd" => app.setup.page = Page::Iodd,
                    "device-advanced" => app.setup.page = Page::Advanced,
                    "mapping-iodd-features" | "offline-profile-ids" => {
                        let xml = include_str!("../../../samples/sensor.xml").replace(
                            "<DeviceFunction>",
                            r#"<DeviceFunction><Features blockParameter="true" dataStorage="false" profileCharacteristic="16384 32768"><SupportedAccessLocks parameter="false" dataStorage="false" localParameterization="true" localUserInterface="true"/></Features>"#,
                        );
                        let catalog = iodd::parse(&xml, "synthetic IODD capability view".into())?;
                        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
                        app.profile.apply_iodd(identity, imported);
                        app.setup.sync(&app.profile);
                        if label == "offline-profile-ids" {
                            app.tab = Tab::Offline;
                            app.workspace.offline = OfflinePage::Parameter;
                            app.offline_parameter.profile_characteristic_payload =
                                "40008000".into();
                        } else {
                            app.tab = Tab::Mappings;
                        }
                    }
                    "mapping-editor" => {
                        app.tab = Tab::Mappings;
                        if let Some((field, origin)) = app.profile.effective_fields().first() {
                            app.setup.edit_field((*field).clone(), *origin != "IODD");
                        }
                    }
                    "layout-condition" | "layout-condition-whole" => {
                        use iolink_workbench::transport::layout_condition::Plan;
                        let whole = label == "layout-condition-whole";
                        let (source, condition) = if whole {
                            (
                                include_str!("../../../samples/sensor-record.xml").replace(
                                    r#"subindexAccessSupported="true""#,
                                    r#"subindexAccessSupported="false""#,
                                ),
                                r#"<ProcessData id="PD"><Condition variableId="V_SIM_Record" subindex="2" value="1"/>"#,
                            )
                        } else {
                            (
                                include_str!("../../../samples/sensor.xml").to_owned(),
                                r#"<ProcessData id="PD"><Condition variableId="V_SIM_Threshold" value="1"/>"#,
                            )
                        };
                        let xml = source.replace(r#"<ProcessData id="PD">"#, condition);
                        let catalog = iodd::parse(&xml, "synthetic condition view".into())?;
                        let (identity, imported) =
                            catalog.select(Some("SIM-SENSOR"), Some("IN"))?;
                        app.profile.apply_iodd(identity, imported);
                        app.setup.sync(&app.profile);
                        app.tab = Tab::Mappings;
                        app.acyclic.store_result(
                            Query::LayoutCondition(Plan::from_profile(&app.profile)?),
                            Ok("Condition matched · expected 1 · observed 1".into()),
                        );
                    }
                    "parameters-raw" => {
                        app.tab = Tab::Parameters;
                        app.workspace.parameter = ParameterPage::Raw;
                        app.acyclic.store_result(
                            Query::Raw(19, 0),
                            Ok("Synthetic raw reply · 53494D".into()),
                        );
                        app.acyclic.store_result(
                            Query::ProductId("SIM-SENSOR".into()),
                            Ok("Matched Product ID SIM-SENSOR".into()),
                        );
                    }
                    "parameters-selected-record" => {
                        app.tab = Tab::Parameters;
                        app.parameter_picker.selected = Some("V_SIM_Record".into());
                        app.parameter_picker.subindex = 1;
                        app.acyclic.store_result(
                            Query::Named {
                                id: "V_SIM_Record".into(),
                                index: 66,
                                subindex: 2,
                            },
                            Ok("Synthetic member reply · 01".into()),
                        );
                    }
                    "integration-mqtt" => {
                        app.tab = Tab::Integration;
                        app.workspace.integration = IntegrationPage::Mqtt;
                        app.mqtt_enabled = true;
                    }
                    "integration-shared-memory" => {
                        app.tab = Tab::Integration;
                        app.workspace.integration = IntegrationPage::SharedMemory;
                        app.shared_memory_enabled = true;
                    }
                    "diagnostics-timing" => {
                        app.tab = Tab::Diagnostics;
                        app.workspace.diagnostics = DiagnosticsPage::Timing;
                    }
                    "master-inspection" => {
                        app.profile =
                            Profile::parse(include_str!("../../../samples/ifm-al1352.toml"))?;
                        app.setup.sync(&app.profile);
                        app.tab = Tab::Diagnostics;
                        app.workspace.diagnostics = DiagnosticsPage::Connection;
                        app.acyclic.store_result(
                            Query::MasterInspection,
                            Ok(Output {
                                text: MASTER_INSPECTION_FIXTURE.into(),
                                port_configuration: Some("Port 8 · Cycle preset automatic · Validation / Data Storage (raw) 0\nConfigured Vendor ID 0 · Configured Device ID 0".into()),
                                comparison_sources: None,
                            }),
                        );
                    }
                    "diagnostics-events" => {
                        app.tab = Tab::Diagnostics;
                        app.workspace.diagnostics = DiagnosticsPage::Events;
                        app.acyclic.store_result(
                            Query::Events,
                            Err(anyhow::anyhow!("Synthetic diagnostics error")),
                        );
                    }
                    "offline-compare" | "offline-signal" | "offline-raw-signal"
                    | "offline-exports" => {
                        app.tab = Tab::Offline;
                        use super::super::saved_capture::Page;
                        app.saved_capture.page = match label {
                            "offline-compare" => {
                                ensure!(app.saved_capture.stage_comparison_drop(
                                    std::path::Path::new(
                                        ".local/synthetic-recordings/Archived baseline 録画"
                                    )
                                ));
                                Page::Compare
                            }
                            "offline-signal" => {
                                app.saved_capture.set_logged_fields(
                                    app.profile.log_fields().into_iter().cloned().collect(),
                                );
                                Page::Signal
                            }
                            "offline-raw-signal" => {
                                app.saved_capture.set_logged_fields(Vec::new());
                                Page::Signal
                            }
                            _ => Page::Exports,
                        };
                    }
                    "writes-form"
                    | "writes-review"
                    | "writes-text-review"
                    | "writes-output-review"
                    | "writes-boolean-output" => {
                        use iolink_workbench::transport::write::{Operation, Plan};
                        app.profile.connection.gateway = Gateway::IfmAl1350;
                        app.profile.connection.endpoint = "https://127.0.0.1:9".into();
                        app.profile.connection.ifm_firmware = "3.1.42".into();
                        app.setup.sync(&app.profile);
                        app.setup.page = Page::Writes;
                        let operation = match label {
                            "writes-review" => Some(Operation::isdu_member(
                                &app.profile,
                                66,
                                2,
                                "FFEC010A41000000".into(),
                                "false",
                            )?),
                            "writes-text-review" => Some(Operation::isdu_typed(
                                &app.profile,
                                66,
                                4,
                                "41424344".into(),
                                "OK",
                            )?),
                            "writes-output-review" => {
                                apply_output_fixture(&mut app.profile)?;
                                app.setup.sync(&app.profile);
                                Some(Operation::process_output_field(
                                    &app.profile,
                                    "0001".into(),
                                    "Enable",
                                    "false",
                                )?)
                            }
                            "writes-boolean-output" => {
                                apply_output_fixture(&mut app.profile)?;
                                let output = app
                                    .profile
                                    .iodd
                                    .as_mut()
                                    .and_then(|import| import.output.as_mut())
                                    .context("missing output fixture")?;
                                output.packed = false;
                                output.bit_length = 1;
                                output.standalone_boolean = true;
                                app.setup.sync(&app.profile);
                                Some(Operation::process_output_field(
                                    &app.profile,
                                    "02".into(),
                                    "Enable",
                                    "false",
                                )?)
                            }
                            _ => None,
                        };
                        if let Some(operation) = operation {
                            app.writes.stage(Plan::new(
                                &app.profile,
                                "SYNTHETIC-MASTER",
                                operation,
                            )?);
                        }
                    }
                    "offline-process" => {
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::Process;
                    }
                    "offline-process-output" => {
                        apply_output_fixture(&mut app.profile)?;
                        let imported = app
                            .profile
                            .iodd
                            .as_mut()
                            .context("missing output fixture")?;
                        imported.layout = "OUT".into();
                        imported.input_bytes = 0;
                        imported.fields.clear();
                        app.profile.input_bytes = 0;
                        app.profile.validate()?;
                        app.setup.sync(&app.profile);
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::Process;
                        app.payload_inspector.output_pasted = "0001".into();
                    }
                    "offline-padded-input"
                    | "offline-boolean-input"
                    | "offline-float-state-name"
                    | "offline-hex-format"
                    | "offline-named-range"
                    | "offline-octet-input"
                    | "offline-text-input"
                    | "offline-temporal-input" => {
                        let input = match label {
                            "offline-boolean-input" => {
                                app.payload_inspector.pasted = "02".into();
                                r#"<ProcessDataIn id="IN" bitLength="1"><Datatype xsi:type="BooleanT"><SingleValue value="true"><Name textId="nominal"/></SingleValue></Datatype><Name textId="switch"/></ProcessDataIn>"#
                            }
                            "offline-float-state-name" => {
                                app.payload_inspector.pasted = "3FA00000".into();
                                r#"<ProcessDataIn id="IN" bitLength="32" gradient="2" displayFormat="Dec.2"><Datatype xsi:type="Float32T"><SingleValue value="1.250e0"><Name textId="nominal"/></SingleValue></Datatype><Name textId="level"/></ProcessDataIn>"#
                            }
                            "offline-hex-format" => {
                                app.payload_inspector.pasted = "FFFC".into();
                                r#"<ProcessDataIn id="IN" bitLength="16" displayFormat="Hex"><Datatype xsi:type="IntegerT" bitLength="16"><SingleValue value="-4"><Name textId="nominal"/></SingleValue></Datatype><Name textId="level"/></ProcessDataIn>"#
                            }
                            "offline-named-range" => {
                                app.payload_inspector.pasted = "0003".into();
                                r#"<ProcessDataIn id="IN" bitLength="16" gradient="2"><Datatype xsi:type="UIntegerT" bitLength="16"><ValueRange lowerValue="2" upperValue="4"><Name textId="nominal"/></ValueRange></Datatype><Name textId="level"/></ProcessDataIn>"#
                            }
                            "offline-octet-input" => {
                                app.payload_inspector.pasted = "0080FF00".into();
                                r#"<ProcessDataIn id="IN" bitLength="32"><Datatype xsi:type="OctetStringT" fixedLength="4"/><Name textId="octets"/></ProcessDataIn>"#
                            }
                            "offline-text-input" => {
                                app.payload_inspector.pasted = "41004200".into();
                                r#"<ProcessDataIn id="IN" bitLength="32"><Datatype xsi:type="StringT" fixedLength="4" encoding="US-ASCII"/><Name textId="text"/></ProcessDataIn>"#
                            }
                            "offline-temporal-input" => {
                                app.payload_inspector.pasted = "9DFF440080000000".into();
                                r#"<ProcessDataIn id="IN" bitLength="64"><Datatype xsi:type="TimeT"/><Name textId="clock"/></ProcessDataIn>"#
                            }
                            _ => {
                                r#"<ProcessDataIn id="IN" bitLength="12"><Datatype xsi:type="UIntegerT" bitLength="12"/><Name textId="integer"/></ProcessDataIn>"#
                            }
                        };
                        let xml = include_str!("../../../samples/sensor.xml").replace(
                            r#"<ProcessDataIn id="IN" bitLength="16"><DatatypeRef datatypeId="PD"/></ProcessDataIn>"#,
                            input,
                        );
                        let catalog = iodd::parse(&xml, format!("synthetic {label} view"))?;
                        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
                        app.profile.apply_iodd(identity, imported);
                        app.setup.sync(&app.profile);
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::Process;
                    }
                    "acyclic-profile" => {
                        let xml = include_str!("../../../samples/sensor.xml").replace(
                            "<ProcessDataIn id=\"IN\" bitLength=\"16\"><DatatypeRef datatypeId=\"PD\"/></ProcessDataIn>",
                            "",
                        );
                        let catalog = iodd::parse(&xml, "synthetic acyclic".into())?;
                        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
                        app.profile.apply_iodd(identity, imported);
                        app.profile.validate()?;
                        app.setup.sync(&app.profile);
                        app.tab = Tab::Mappings;
                    }
                    "diagnostics-device-profile" => {
                        let snapshot = iolink_workbench::diagnostics::DeviceProfileSnapshot::read(
                            &Profile::default(),
                        )?;
                        let review = snapshot.review()?;
                        ensure!(review.ok(), "synthetic profile read must decode");
                        app.acyclic
                            .store_result(Query::DeviceProfile, Ok(review.text().into()));
                        app.tab = Tab::Diagnostics;
                        app.workspace.diagnostics = DiagnosticsPage::DeviceProfile;
                    }
                    "offline-process-descriptor" => {
                        let catalog = iodd::parse(
                            include_str!("../../../samples/sensor.xml"),
                            "synthetic process descriptor view".into(),
                        )?;
                        let (identity, imported) = catalog.select(Some("SIM-SENSOR"), None)?;
                        app.profile.apply_iodd(identity, imported);
                        app.setup.sync(&app.profile);
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::Parameter;
                        app.offline_parameter.process_descriptor_payload = "010100030F01".into();
                    }
                    "offline-descriptor-plan" => {
                        app.profile = Profile::default();
                        app.setup.sync(&app.profile);
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::Parameter;
                        app.offline_parameter.process_descriptor_payload = "010200030E02".into();
                    }
                    "offline-parameter" => {
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::Parameter;
                        app.offline_parameter.picker.selected = Some("V_SIM_Threshold".into());
                    }
                    "offline-snapshot" => {
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::Snapshot;
                    }
                    "offline-events" => {
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::Events;
                        app.offline_diagnostics.stage_error_type_fixture();
                    }
                    "offline-master-snapshot" => {
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::DiagnosticSnapshots;
                        app.acyclic.store_result(
                            Query::SavedMasterInspection(".local/synthetic-master.json".into()),
                            Ok(Output {
                                text: "Saved profile Synthetic bench · Endpoint http://127.0.0.1:1"
                                    .into(),
                                port_configuration: Some("Port 1 · Cycle preset automatic".into()),
                                comparison_sources: None,
                            }),
                        );
                    }
                    "offline-master-comparison" => {
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::DiagnosticSnapshots;
                        app.acyclic.store_result(
                            Query::CompareMasterSnapshots(
                                ".local/synthetic-master-left.json".into(),
                                ".local/synthetic-master-right.json".into(),
                            ),
                            Ok(Output {
                                text: "Saved master comparison · left → right\nPort 1 · Configured Vendor ID 310 → 311".into(),
                                port_configuration: None,
                                comparison_sources: Some([
                                    "Left recorded master supply".into(),
                                    "Right recorded master supply".into(),
                                ]),
                            }),
                        );
                    }
                    "offline-diagnostic-snapshot" | "offline-diagnostic-comparison" => {
                        app.tab = Tab::Offline;
                        app.workspace.offline = OfflinePage::DiagnosticSnapshots;
                        app.saved_diagnostics
                            .stage_render_fixture(label == "offline-diagnostic-comparison");
                    }
                    "profiles-dialog" => app.workspace.profile_dialog = true,
                    "profiles-files" => {
                        app.workspace.profile_dialog = true;
                        app.setup.profiles_page = super::super::setup::ProfilePage::Files;
                    }
                    "drop-preview" => {}
                    "drop-saved-preview" => app.tab = Tab::Offline,
                    "latest-recording" => {
                        app.tab = Tab::Logging;
                        app.latest_recording = Some(super::super::Recording {
                            directory: ".local/synthetic-recording".into(),
                            rows: 1250,
                            parts: 2,
                            unconfirmed_rows: 6,
                            has_error: true,
                        });
                    }
                    "port-capture-review" | "port-capture-results" | "port-status-results" => {
                        app.tab = Tab::Logging;
                        app.workspace.logging = LoggingPage::Ports;
                        app.port_capture
                            .stage_render_fixture(profile.clone(), label == "port-capture-results");
                        if label == "port-status-results" {
                            app.port_capture.stage_status_fixture();
                        }
                    }
                    "connection-dialog" => app.workspace.connect_dialog = true,
                    "connection-needs-endpoint"
                    | "al1351-connection"
                    | "al1352-connection"
                    | "al1353-connection" => {
                        app.profile = Profile::parse(match label {
                            "al1351-connection" => include_str!("../../../samples/ifm-al1351.toml"),
                            "al1352-connection" => include_str!("../../../samples/ifm-al1352.toml"),
                            "al1353-connection" => include_str!("../../../samples/ifm-al1353.toml"),
                            _ => include_str!("../../../samples/ifm-al1350.toml"),
                        })?;
                        if label != "connection-needs-endpoint" {
                            app.profile.connection.port = app.profile.connection.gateway.max_port();
                            app.profile.connection.endpoint = "http://127.0.0.1:1".into();
                            app.profile.validate()?;
                        }
                        app.setup.sync(&app.profile);
                        app.workspace.connect_dialog = true;
                    }
                    "connection-feed-settings" => {
                        app.api_enabled = true;
                        app.mqtt_enabled = true;
                        app.mqtt_topic = "lab/+/input".into();
                        app.shared_memory_enabled = true;
                        app.workspace.connect_dialog = true;
                    }
                    _ => unreachable!(),
                }
                views.push((name, label, app, None, None));
            }
        }
        if name == "time" {
            use iolink_workbench::transport::write::{Operation, Plan};
            let mut app = App::new();
            app.profile = profile.clone();
            app.profile.connection.gateway = Gateway::IfmAl1350;
            app.profile.connection.endpoint = "https://127.0.0.1:9".into();
            app.profile.connection.ifm_firmware = "3.1.42".into();
            app.profile
                .iodd
                .as_mut()
                .and_then(|iodd| {
                    iodd.parameters
                        .iter_mut()
                        .find(|parameter| parameter.index == 71)
                })
                .context("missing synthetic time parameter")?
                .access = iodd::Access::ReadWrite;
            app.setup.sync(&app.profile);
            app.setup.page = super::super::setup::Page::Writes;
            let operation = Operation::isdu_typed(
                &app.profile,
                71,
                0,
                "9DFF440000000000".into(),
                "1984-01-01T00:00:00.001Z",
            )?;
            app.writes
                .stage(Plan::new(&app.profile, "SYNTHETIC-MASTER", operation)?);
            views.push((name, "writes-time-review", app, None, None));
        }
    }

    for (name, state, snapshots) in [
        ("observation-recent", State::Recent, false),
        ("observation-held", State::HostSilence, false),
        ("observation-stopped", State::Stopped, false),
        ("observation-invalid", State::InvalidAtReceipt, false),
        ("observation-update-old", State::UpdateOld, false),
        ("observation-snapshot-unknown", State::Recent, true),
    ] {
        let (app, gate) = observation_fixture(state, snapshots)?;
        views.push((name, "Inputs", app, gate, Some(state)));
    }
    {
        let (mut app, gate) = observation_fixture(State::Stopped, true)?;
        let view = Observation::at(&app.live, false, Instant::now(), &app.profile.connection)
            .context("synthetic copied observation")?;
        app.payload_inspector.copy(
            app.live.input.context("synthetic copied bytes")?,
            view,
            &app.live,
            &app.profile,
        );
        app.tab = Tab::Offline;
        app.workspace.offline = super::super::workspace::OfflinePage::Process;
        views.push((
            "observation-copied-snapshot",
            "Process bytes",
            app,
            gate,
            None,
        ));
    }
    let mut cases = Vec::new();
    for (name, label, mut app, _gate, expected_observation) in views {
        let gated_session = app.worker.is_some();
        let before = app.profile.clone();
        let mut case = Case {
            profile: name,
            tab: label,
            width: SIZE[0],
            height: SIZE[1],
            passes: 0,
            shapes: 0,
            meshes: 0,
            triangles: 0,
        };
        for pass in 0..2 {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(SIZE[0] as f32, SIZE[1] as f32),
                )),
                time: Some(cases.len() as f64 + f64::from(pass) / 60.0),
                focused: false,
                ..Default::default()
            };
            if matches!(label, "drop-preview" | "drop-saved-preview") {
                input.hovered_files.push(egui::HoveredFile::default());
            }
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .native_pixels_per_point = Some(1.0);
            ensure!(
                input.events.is_empty(),
                "self-test must not inject input events"
            );
            let output = ctx.run(input, |ctx| app.render(ctx));
            let review_note = match label {
                "writes-review" => Some("Changed IODD members (1)"),
                "writes-output-review" => Some("IODD output · OUT · 13 bits"),
                "writes-boolean-output" => Some(
                    "Standalone Boolean byte 02 is noncanonical. Workbench interprets it as true.",
                ),
                "offline-process-output" => Some("true (raw true, bits 0x1)"),
                "offline-padded-input" => Some(
                    "Standalone integer input has noncanonical padding. The IODD value uses the declared low bits.",
                ),
                "offline-boolean-input" => Some(
                    "Standalone Boolean input has a noncanonical byte. The IODD interprets any nonzero byte as true.",
                ),
                "diagnostics-device-profile" => Some("Read device profile"),
                "offline-descriptor-plan" => Some(
                    "Suggested manual fields · 3\nInput width · 2 bytes\nInput bit 0 · Bool · width 1 · offset 0\nInput bit 1 · Bool · width 1 · offset 1\nInput bit 2 · Signed · width 14 · offset 2",
                ),
                "offline-process-descriptor" => Some(
                    "Descriptor matches mapped IODD fields\nRaw reply · 010100030F01\n1 · Set of BooleanT (0x01) · 1 bits at bit 0\n2 · IntegerT (0x03) · 15 bits at bit 1\nSelected IODD layout · IN · 2 bytes\nlevel [1] · IntegerT · 15 bits at bit 1 · Match\nswitch [2] · Set of BooleanT · 1 bits at bit 0 · Match",
                ),
                "offline-profile-ids" => Some(
                    "Profile IDs match the IODD\nRaw reply · 40008000\nIODD profile IDs · 0x4000 (16384), 0x8000 (32768)\nReply profile IDs · 0x4000 (16384), 0x8000 (32768)",
                ),
                "offline-float-state-name" => Some("Nominal setting (2.50)"),
                "offline-hex-format" => Some("Nominal setting (FFFCh)"),
                "offline-named-range" => Some("Nominal setting (6)"),
                "offline-octet-input" => Some("0x00,0x80,0xFF,0x00"),
                "offline-text-input" => {
                    Some("Invalid StringT (nonzero string padding follows the first NUL)")
                }
                "offline-temporal-input" => Some("1984-01-01 00:00:00.500 UTC"),
                "layout-condition" => Some("Condition matched · expected 1 · observed 1"),
                "mapping-iodd-features" => Some(
                    "Block parameter transfer · Yes\nDevice Data Storage · No\nProfile IDs · 0x4000 (16384), 0x8000 (32768)\nAccess lock support · Parameter No · Data Storage No · Local parameterization Yes · Local user interface Yes",
                ),
                "layout-condition-whole" => Some("Reads the whole value for selector subindex 2"),
                "acyclic-profile" => Some("Acyclic IODD layout"),
                "offline-master-snapshot" => {
                    Some("Saved profile Synthetic bench · Endpoint http://127.0.0.1:1")
                }
                "offline-master-comparison" => Some("Right recorded master supply"),
                "offline-diagnostic-snapshot" => Some("37:0 · synthetic fixture reply unavailable"),
                "offline-diagnostic-comparison" => Some("Wire slot positions"),
                "al1351-connection" => Some("ifm AL1351 · http://127.0.0.1:1"),
                "al1352-connection" => Some("ifm AL1352 · http://127.0.0.1:1"),
                "al1353-connection" => Some("ifm AL1353 · http://127.0.0.1:1"),
                "master-inspection" => Some(MASTER_INSPECTION_FIXTURE),
                "offline-events" => Some(
                    "ErrorType 0x8011 · Standard device application error\nIndex is unavailable",
                ),
                "port-capture-review" => Some("Start capture set"),
                "port-capture-results" => Some("Synthetic device identity mismatch"),
                "port-status-results" => Some("Synthetic status request timed out"),
                "writes-text-review" => {
                    Some("Readback accepts the same IODD text with different zero padding.")
                }
                "writes-time-review" => Some(
                    "Typed input uses milliseconds. Fractions round by less than one tick. See exact ticks in the details.",
                ),
                _ => None,
            };
            if let Some(review_note) = review_note {
                let mut texts = Vec::new();
                for shape in &output.shapes {
                    text_shapes(&shape.shape, &mut texts);
                }
                ensure!(
                    texts.contains(&review_note),
                    "required note is missing from {label}"
                );
                if label == "master-inspection" {
                    ensure!(
                        texts.contains(&"Port configuration"),
                        "configuration control is missing"
                    );
                    ensure!(
                        texts
                            .iter()
                            .any(|text| text.contains("Configured Vendor ID 0")),
                        "configured port settings are missing from the expanded view"
                    );
                }
                if label == "offline-boolean-input" {
                    ensure!(
                        texts.contains(&"Nominal setting (true)"),
                        "named process value is missing"
                    );
                }
                if label == "offline-diagnostic-comparison" {
                    ensure!(
                        texts.iter().any(|text| text.contains("E48DFE · 1 → 0")),
                        "recorded pending-event count change is missing"
                    );
                }
            }
            if let Some(expected) = expected_observation {
                let observation = Observation::at(
                    &app.live,
                    gated_session,
                    Instant::now(),
                    &app.profile.connection,
                )
                .context("missing synthetic observation")?;
                ensure!(
                    observation.state == expected,
                    "unexpected observation state in {name}"
                );
                let mut texts = Vec::new();
                for shape in &output.shapes {
                    text_shapes(&shape.shape, &mut texts);
                }
                ensure!(
                    texts.contains(&"RAW  03E9"),
                    "raw observation missing in {name}"
                );
                ensure!(
                    texts.contains(&"50.0") == observation.show_typed(),
                    "typed observation visibility differs in {name}"
                );
                ensure!(
                    texts.contains(&expected.label()),
                    "observation label missing in {name}"
                );
            }
            ensure!(
                output.pixels_per_point == 1.0,
                "self-test requires nominal one-pixel-per-point rendering"
            );
            let shapes = output.shapes.len();
            let primitives = ctx.tessellate(output.shapes.clone(), output.pixels_per_point);
            let mut meshes = 0;
            let mut triangles = 0;
            for primitive in &primitives {
                match &primitive.primitive {
                    egui::epaint::Primitive::Mesh(mesh) => {
                        ensure!(mesh.is_valid(), "GUI produced an invalid mesh");
                        meshes += 1;
                        triangles += mesh.indices.len() / 3;
                    }
                    egui::epaint::Primitive::Callback(_) => {
                        anyhow::bail!("unexpected native callback in offscreen GUI self-test")
                    }
                }
            }
            ensure!(
                shapes > 0 && meshes > 0 && triangles > 0,
                "GUI view produced no drawable geometry"
            );
            ensure!(
                app.profile == before,
                "rendering changed the applied profile"
            );
            ensure!(
                app.worker.is_some() == gated_session && !app.acyclic.busy() && !app.setup.busy(),
                "rendering dispatched a worker"
            );
            paint(SIZE, &output, &primitives)?;
            case.passes += 1;
            case.shapes += shapes;
            case.meshes += meshes;
            case.triangles += triangles;
        }
        cases.push(case);
    }
    Ok(cases)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_built_in_views_render_without_input_or_profile_changes() -> Result<()> {
        let cases = render(&Context::default(), |_, _, _| Ok(()))?;
        assert_eq!(cases.len(), 132);
        assert!(
            cases
                .iter()
                .all(|case| case.passes == 2 && case.triangles > 0)
        );
        Ok(())
    }
}
