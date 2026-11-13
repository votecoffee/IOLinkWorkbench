use super::*;
use std::thread;
use windows_sys::Win32::Security::{
    Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
    },
    DACL_SECURITY_INFORMATION,
};

fn config() -> Config {
    Config {
        name: format!(
            "native_{}_{}",
            std::process::id(),
            crate::shared_memory::INSTANCE.fetch_add(1, Ordering::Relaxed)
        ),
        ..Default::default()
    }
}
const DATA:&[u8]=br#"{"format_version":1,"snapshot_sequence":1,"sampling_active":true,"gateway_connected_last_observed":false}"#;

#[test]
fn protected_dacls_have_only_two_allow_entries_and_read_handle_denies_write_view() -> Result<()> {
    let config = config();
    let mapping = Mapping::create(&config)?;
    for handle in [mapping._mapping.0, mapping.mutex.0] {
        let mut descriptor = ptr::null_mut();
        ensure!(
            unsafe {
                GetSecurityInfo(
                    handle,
                    SE_KERNEL_OBJECT,
                    DACL_SECURITY_INFORMATION,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    ptr::null_mut(),
                    &mut descriptor,
                )
            } == 0,
            "GetSecurityInfo failed"
        );
        let _memory = Local(descriptor);
        let mut string = ptr::null_mut();
        ensure!(
            unsafe {
                ConvertSecurityDescriptorToStringSecurityDescriptorW(
                    descriptor,
                    SDDL_REVISION_1,
                    DACL_SECURITY_INFORMATION,
                    &mut string,
                    ptr::null_mut(),
                )
            } != 0,
            "SDDL conversion failed"
        );
        let _string = Local(string.cast());
        let mut length = 0;
        while length < 1024 && unsafe { *string.add(length) } != 0 {
            length += 1;
        }
        assert!(length < 1024);
        let sddl = String::from_utf16(unsafe { std::slice::from_raw_parts(string, length) })?;
        assert!(sddl.starts_with("D:P"));
        assert_eq!(sddl.matches("(A;").count(), 2);
        assert!(sddl.contains(";;;SY)"));
        assert!(!sddl.contains(";;;WD)"));
        assert!(!sddl.contains(";;;BA)"));
    }
    let name = wide(&config.mapping_name());
    let read_handle =
        OwnedHandle::new(unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) })?;
    let denied = unsafe { MapViewOfFile(read_handle.0, FILE_MAP_WRITE, 0, 0, MAP_BYTES) };
    assert!(denied.Value.is_null());
    Ok(())
}

fn abandon(config: &Config) -> Result<()> {
    let name = wide(&config.mutex_name());
    thread::spawn(move || -> Result<()> {
        let mutex = OwnedHandle::new(unsafe {
            OpenMutexW(
                SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
                0,
                name.as_ptr(),
            )
        })?;
        let (guard, _) = lock(&mutex, 0)?.context("test mutex unavailable")?;
        std::mem::forget(guard);
        Ok(())
    })
    .join()
    .expect("abandon helper")?;
    Ok(())
}

#[test]
fn reader_rejects_abandonment_and_writer_recovers_by_full_commit() -> Result<()> {
    let config = config();
    let mut mapping = Mapping::create(&config)?;
    assert!(mapping.write(DATA, 1, 1)?.is_some());
    abandon(&config)?;
    assert!(read(&config, 0, 1000).is_err());
    abandon(&config)?;
    assert_eq!(mapping.write(DATA, 1, 1)?, Some(true));
    assert_eq!(read(&config, 50, 1000)?.header.publish_sequence, 1);
    Ok(())
}

#[test]
fn busy_reader_causes_zero_wait_publisher_skip() -> Result<()> {
    let config = config();
    let mut mapping = Mapping::create(&config)?;
    assert!(mapping.write(DATA, 1, 1)?.is_some());
    let name = wide(&config.mutex_name());
    let (ready_rx, release_tx, worker) = {
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = thread::spawn(move || -> Result<()> {
            let mutex = OwnedHandle::new(unsafe {
                OpenMutexW(
                    SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
                    0,
                    name.as_ptr(),
                )
            })?;
            let (_guard, _) = lock(&mutex, 0)?.context("test mutex unavailable")?;
            ready_tx.send(())?;
            release_rx.recv_timeout(std::time::Duration::from_secs(2))?;
            Ok(())
        });
        (ready_rx, release_tx, worker)
    };
    ready_rx.recv_timeout(std::time::Duration::from_secs(2))?;
    assert_eq!(mapping.write(DATA, 1, 1)?, None);
    release_tx.send(())?;
    worker.join().expect("hold helper")?;
    assert_eq!(read(&config, 50, 1000)?.header.publish_sequence, 1);
    Ok(())
}
