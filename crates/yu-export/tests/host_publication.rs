use std::fs;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use yu_export::portable::{Destination, HostPublication};

#[test]
fn host_staging_publishes_new_and_existing_outputs() {
    for existing in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let target = root.path().join("output.pdf");
        if existing {
            fs::write(&target, b"old output").unwrap();
        }
        let mut destination = Destination::capture(&target, existing).unwrap();
        let stage_path = staging.path().to_path_buf();
        destination.set_publication(Some(Arc::new(HostPublication {
            staging_directory: stage_path.clone(),
            replace: Box::new(move |source, target, overwrite| {
                assert_eq!(source.parent(), Some(stage_path.as_path()));
                assert_eq!(overwrite, existing);
                assert_eq!(fs::read(source).unwrap(), b"complete output");
                fs::rename(source, target).map_err(|e| e.to_string())
            }),
        })));
        destination
            .publish(b"complete output", &[], || Ok(()))
            .unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"complete output");
        assert_eq!(fs::read_dir(staging.path()).unwrap().count(), 0);
    }
}

#[test]
fn host_failure_keeps_existing_output_and_cleans_staging() {
    let root = tempfile::tempdir().unwrap();
    let staging = tempfile::tempdir().unwrap();
    let target = root.path().join("output.pdf");
    fs::write(&target, b"old output").unwrap();
    let mut destination = Destination::capture(&target, true).unwrap();
    destination.set_publication(Some(Arc::new(HostPublication {
        staging_directory: staging.path().to_path_buf(),
        replace: Box::new(|_, _, _| Err("permission denied".into())),
    })));
    assert!(destination.publish(b"new output", &[], || Ok(())).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"old output");
    assert_eq!(fs::read_dir(staging.path()).unwrap().count(), 0);
}

#[test]
fn conflict_and_cancel_do_not_call_host_or_change_output() {
    for conflict in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        let target = root.path().join("output.pdf");
        let mut destination = Destination::capture(&target, false).unwrap();
        if conflict {
            fs::write(&target, b"external output").unwrap();
        }
        let called = Arc::new(AtomicBool::new(false));
        let flag = called.clone();
        destination.set_publication(Some(Arc::new(HostPublication {
            staging_directory: staging.path().to_path_buf(),
            replace: Box::new(move |_, _, _| {
                flag.store(true, Ordering::SeqCst);
                Ok(())
            }),
        })));
        assert!(
            destination
                .publish_guarded(
                    b"new output",
                    &[],
                    || Ok(()),
                    || Err::<(), _>("cancelled".into())
                )
                .is_err()
        );
        assert!(!called.load(Ordering::SeqCst));
        if conflict {
            assert_eq!(fs::read(&target).unwrap(), b"external output");
        } else {
            assert!(!target.exists());
        }
        assert_eq!(fs::read_dir(staging.path()).unwrap().count(), 0);
    }
}
