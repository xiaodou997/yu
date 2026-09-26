use yu_assets::{EmbeddedRenderRequest, EmbeddedResourceKind};
use yu_core::{ByteOffset, Revision, TextRange};
use yu_embedded_client::{NativeRendererClient, RenderControl, RenderFailure};

fn request(revision: u64, kind: EmbeddedResourceKind, source: &str) -> EmbeddedRenderRequest {
    EmbeddedRenderRequest::new(
        Revision::new(revision),
        TextRange::new(ByteOffset::ZERO, ByteOffset::new(source.len() as u64)).expect("range"),
        kind,
        source,
    )
    .expect("request")
}
#[test]
fn app_client_uses_one_real_helper_and_rejects_stale_requests() {
    let mut client = NativeRendererClient::new(
        std::env::var_os("YU_TEST_RENDERER")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_yu-document-renderer").into()),
        97,
    );
    let control = RenderControl::default();
    control.set_revision(4);
    assert!(client.process_id().is_none());
    let first = client
        .render(
            &request(4, EmbeddedResourceKind::Math, r"\frac{a}{b}"),
            &control,
        )
        .expect("native math");
    assert!(first.markup().expect("svg").contains("<path"));
    let pid = client.process_id().expect("helper launched");
    let diagram = client
        .render(
            &request(4, EmbeddedResourceKind::Mermaid, "flowchart LR\nA-->B"),
            &control,
        )
        .expect("native diagram");
    assert!(diagram.markup().expect("svg").contains("<svg"));
    assert_eq!(client.process_id(), Some(pid));
    control.set_revision(5);
    assert_eq!(
        client.render(&request(4, EmbeddedResourceKind::Math, "x"), &control),
        Err(RenderFailure::Cancelled)
    );
    let invalid = client.render(
        &request(5, EmbeddedResourceKind::Math, r"\notARealYuCommand{x}"),
        &control,
    );
    assert!(matches!(invalid, Err(RenderFailure::InvalidSource(message)) if !message.is_empty()));
    control.close();
    assert_eq!(
        client.render(&request(5, EmbeddedResourceKind::Math, "x"), &control),
        Err(RenderFailure::Cancelled)
    );
}

#[test]
fn gantt_calendar_changes_at_same_revision_reach_real_helper_and_recover() {
    let mut client = NativeRendererClient::new(
        std::env::var_os("YU_TEST_RENDERER")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_yu-document-renderer").into()),
        109,
    );
    let control = RenderControl::default();
    control.set_revision(7);
    let source = "gantt\ndateFormat HH:mm\naxisFormat %Y-%m-%d %H:%M\nA :a,08:30,1h";
    let make = |day, source: &str| {
        request(7, EmbeddedResourceKind::Mermaid, source)
            .with_style(yu_assets::EmbeddedStyle::default().with_reference_day(Some(day)))
    };
    let a = client
        .render(&make(20454, source), &control)
        .expect("first local day");
    let pid = client.process_id().expect("helper started");
    let b = client
        .render(&make(20455, source), &control)
        .expect("next local day");
    assert!(a.markup().expect("first SVG").contains("2026-01-01"));
    assert!(b.markup().expect("next SVG").contains("2026-01-02"));
    assert_ne!(a.markup(), b.markup());
    let invalid = client.render(
        &make(20455, "gantt\ndateFormat HH:mm\nA :a,24:00,1h"),
        &control,
    );
    assert!(matches!(invalid, Err(RenderFailure::InvalidSource(_))));
    let restored = client
        .render(&make(20454, source), &control)
        .expect("clock rollback and error recovery");
    assert_eq!(restored.markup(), a.markup());
    assert_eq!(client.process_id(), Some(pid));
    control.close();
    assert_eq!(
        client.render(&make(20455, source), &control),
        Err(RenderFailure::Cancelled)
    );
}

#[test]
fn central_lifecycle_errors_do_not_poison_the_real_helper_or_request_identity() {
    let mut client = NativeRendererClient::new(
        std::env::var_os("YU_TEST_RENDERER")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| env!("CARGO_BIN_EXE_yu-document-renderer").into()),
        108,
    );
    let control = RenderControl::default();
    let valid = "sequenceDiagram\nparticipant A\ncreate actor B as 工作🙂\nA()->>()B: Start\nactivate B\nB()->>()B: Work\ndeactivate B\ndestroy B\nB()->>A: Finish";
    for revision in [11, 12] {
        control.set_revision(revision);
        let rendered = client
            .render(
                &request(revision, EmbeddedResourceKind::Mermaid, valid),
                &control,
            )
            .expect("central lifecycle in real helper");
        assert_eq!(
            rendered
                .markup()
                .expect("vector source")
                .matches("class=\"sequence-central-connection\"")
                .count(),
            5
        );
        let pid = client.process_id().expect("live helper");
        let invalid = client.render(
            &request(
                revision,
                EmbeddedResourceKind::Mermaid,
                "sequenceDiagram\nA->>()B: not activation\ndeactivate B",
            ),
            &control,
        );
        assert!(matches!(invalid, Err(RenderFailure::InvalidSource(message))
            if message.contains("inactive participant")));
        let recovered = client
            .render(
                &request(revision, EmbeddedResourceKind::Mermaid, valid),
                &control,
            )
            .expect("valid request after rejected lifetime");
        assert_eq!(recovered.markup(), rendered.markup());
        assert_eq!(client.process_id(), Some(pid));
        assert_eq!(
            client.render(
                &request(revision - 1, EmbeddedResourceKind::Mermaid, valid),
                &control,
            ),
            Err(RenderFailure::Cancelled)
        );
    }
    control.close();
    assert_eq!(
        client.render(&request(12, EmbeddedResourceKind::Mermaid, valid), &control),
        Err(RenderFailure::Cancelled)
    );
}

#[cfg(unix)]
#[test]
fn revision_change_cancels_a_running_child_without_waiting_for_its_output() {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        sync::mpsc,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    let directory = std::env::temp_dir().join(format!(
        "yu-render-cancel-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir(&directory).expect("directory");
    let marker = directory.join("started");
    let script = directory.join("renderer");
    let quoted_marker = marker.to_string_lossy().replace('\'', "'\\''");
    // A deterministic single-process fixture: after accepting a request it
    // never writes a response. The test can observe exactly when work started.
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nread request\nprintf '%s' \"$$\" > '{quoted_marker}'\nwhile :; do :; done\n"
        ),
    )
    .expect("fixture");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).expect("executable");
    let control = RenderControl::default();
    control.set_revision(7);
    let worker_control = control.clone();
    let (sender, receiver) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let mut client = NativeRendererClient::new(script, 27);
        let result = client.render(
            &request(7, EmbeddedResourceKind::Math, "x"),
            &worker_control,
        );
        let _ = sender.send((result, client.process_id()));
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    while !marker.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = marker.exists();
    control.set_revision(8);
    let (result, pid) = receiver
        .recv_timeout(Duration::from_secs(3))
        .expect("cancellation response");
    thread.join().expect("worker");
    assert!(
        started,
        "must test in-flight cancellation, not a pre-start rejection"
    );
    assert_eq!(result, Err(RenderFailure::Cancelled));
    assert!(pid.is_none());
    fs::remove_dir_all(directory).expect("cleanup");
}

#[cfg(unix)]
#[test]
fn exited_helper_is_reaped_without_another_request_or_client_drop() {
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        process::Command,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };
    let directory = std::env::temp_dir().join(format!(
        "yu-render-reap-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir(&directory).expect("directory");
    let marker = directory.join("pid");
    let script = directory.join("renderer");
    let quoted_marker = marker.to_string_lossy().replace('\'', "'\\''");
    fs::write(&script, format!("#!/bin/sh\nread request\nprintf '%s' \"$$\" > '{quoted_marker}'\nprintf '%s\\n' '{{\"id\":1,\"document\":17,\"revision\":0,\"status\":\"failed\",\"diagnostic\":\"fixture\"}}'\n")).expect("script");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).expect("permissions");
    let mut client = NativeRendererClient::new(script, 17);
    assert!(matches!(
        client.render(
            &request(0, EmbeddedResourceKind::Math, "x"),
            &RenderControl::default()
        ),
        Err(RenderFailure::InvalidSource(_))
    ));
    let pid = fs::read_to_string(marker).expect("pid");
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        // Observe the OS, not process_id/try_wait: those could reap the child
        // themselves and mask a missing idle cleanup path.
        let status = Command::new("ps")
            .args(["-p", pid.trim(), "-o", "stat="])
            .output()
            .expect("ps");
        if status.stdout.is_empty() {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "helper remains in process table: {}",
            String::from_utf8_lossy(&status.stdout)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(client.process_id().is_none());
    drop(client);
    fs::remove_dir_all(directory).expect("cleanup");
}

#[cfg(unix)]
#[test]
#[ignore = "real 60-second helper idle lifecycle; run explicitly for release acceptance"]
fn real_helper_idle_exit_is_reaped_and_next_request_restarts() {
    use std::{
        process::Command,
        time::{Duration, Instant},
    };
    let binary = std::env::var_os("YU_TEST_RENDERER")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_yu-document-renderer").into());
    let mut client = NativeRendererClient::new(binary, 101);
    let control = RenderControl::default();
    client
        .render(&request(0, EmbeddedResourceKind::Math, "x"), &control)
        .expect("first render");
    let first_pid = client.process_id().expect("live helper");
    let start = Instant::now();
    loop {
        let status = Command::new("ps")
            .args(["-p", &first_pid.to_string(), "-o", "stat="])
            .output()
            .expect("ps");
        if status.stdout.is_empty() {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(65),
            "idle helper remains: {}",
            String::from_utf8_lossy(&status.stdout)
        );
        std::thread::sleep(Duration::from_secs(1));
    }
    assert!(start.elapsed() >= Duration::from_secs(58));
    assert!(client.process_id().is_none());
    client
        .render(&request(0, EmbeddedResourceKind::Math, "y"), &control)
        .expect("restart render");
    let next_pid = client.process_id().expect("new helper");
    assert_ne!(first_pid, next_pid);
    println!(
        "idle helper {first_pid} reaped after {:.3}s; next request started {next_pid}",
        start.elapsed().as_secs_f64()
    );
}
