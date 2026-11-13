//! Windows handles and mapped bytes are owned here; all byte access holds the named mutex.
#[cfg(test)]
mod tests;
use super::{
    Config, Readout,
    layout::{HEADER_BYTES, Header, MAP_BYTES},
};
use anyhow::{Context, Result, ensure};
use std::{
    ptr,
    sync::atomic::{Ordering, fence},
};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ALREADY_EXISTS, FILETIME, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
        LocalFree, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    },
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SDDL_REVISION_1,
        },
        GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::{
        Memory::{
            CreateFileMappingW, FILE_MAP_READ, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS,
            MapViewOfFile, OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
        },
        Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
        Threading::{
            CreateMutexW, GetCurrentProcess, GetProcessTimes, MUTEX_MODIFY_STATE, OpenMutexW,
            OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
            ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE, WaitForSingleObject,
        },
    },
};

struct OwnedHandle(HANDLE);
impl OwnedHandle {
    fn new(raw: HANDLE) -> Result<Self> {
        ensure!(
            !raw.is_null(),
            "Windows IPC: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self(raw))
    }
}
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Local(*mut core::ffi::c_void);
impl Drop for Local {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
struct View(MEMORY_MAPPED_VIEW_ADDRESS);
impl Drop for View {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.0);
        }
    }
}
struct Lock<'a>(&'a OwnedHandle);
impl Drop for Lock<'_> {
    fn drop(&mut self) {
        fence(Ordering::SeqCst);
        unsafe {
            ReleaseMutex(self.0.0);
        }
    }
}
fn lock(handle: &OwnedHandle, wait_ms: u32) -> Result<Option<(Lock<'_>, bool)>> {
    match unsafe { WaitForSingleObject(handle.0, wait_ms) } {
        WAIT_OBJECT_0 => {
            fence(Ordering::SeqCst);
            Ok(Some((Lock(handle), false)))
        }
        WAIT_ABANDONED => {
            fence(Ordering::SeqCst);
            Ok(Some((Lock(handle), true)))
        }
        WAIT_TIMEOUT => Ok(None),
        _ => anyhow::bail!("Windows mutex wait: {}", std::io::Error::last_os_error()),
    }
}
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

fn descriptor() -> Result<Local> {
    let mut raw = ptr::null_mut();
    ensure!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } != 0,
        "OpenProcessToken failed"
    );
    let token = OwnedHandle::new(raw)?;
    let mut size = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut size);
    }
    ensure!((1..=65536).contains(&size), "invalid token size");
    let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
    ensure!(
        unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            )
        } != 0,
        "GetTokenInformation failed"
    );
    // TOKEN_USER and its SID point into the aligned token-information buffer.
    let sid = unsafe { (*(buffer.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut sid_text = ptr::null_mut();
    ensure!(
        unsafe { ConvertSidToStringSidW(sid, &mut sid_text) } != 0,
        "ConvertSidToStringSid failed"
    );
    let _sid_memory = Local(sid_text.cast());
    let mut length = 0;
    while length < 256 && unsafe { *sid_text.add(length) } != 0 {
        length += 1;
    }
    ensure!(length < 256, "SID string exceeds bounds");
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(sid_text, length) })?;
    let sddl = wide(&format!("D:P(A;;GA;;;SY)(A;;GA;;;{sid})"));
    let mut security = ptr::null_mut();
    ensure!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut security,
                ptr::null_mut(),
            )
        } != 0,
        "security descriptor conversion failed"
    );
    Ok(Local(security))
}

fn creation_time(handle: HANDLE) -> Result<u64> {
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    ensure!(
        unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) } != 0,
        "GetProcessTimes failed"
    );
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}
pub(super) fn qpc() -> Result<(u64, u64)> {
    let mut ticks = 0;
    let mut frequency = 0;
    ensure!(
        unsafe { QueryPerformanceCounter(&mut ticks) } != 0
            && unsafe { QueryPerformanceFrequency(&mut frequency) } != 0
            && ticks >= 0
            && frequency > 0,
        "QPC unavailable"
    );
    Ok((ticks as u64, frequency as u64))
}

pub(super) struct Mapping {
    mutex: OwnedHandle,
    _mapping: OwnedHandle,
    view: View,
    created: u64,
}
// Ownership moves to one publisher worker. No Rust references to mapped bytes escape,
// and all accesses use the same kernel mutex with acquire/release ordering.
unsafe impl Send for Mapping {}
impl Mapping {
    pub(super) fn create(config: &Config) -> Result<Self> {
        let security = descriptor()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: security.0,
            bInheritHandle: 0,
        };
        let mutex_name = wide(&config.mutex_name());
        let mapping_name = wide(&config.mapping_name());
        let raw = unsafe { CreateMutexW(&attributes, 1, mutex_name.as_ptr()) };
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let mutex = OwnedHandle::new(raw)?;
        ensure!(
            !exists,
            "shared-memory mutex name already exists; choose another name or close old readers"
        );
        let guard = Lock(&mutex);
        let raw = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                &attributes,
                PAGE_READWRITE,
                0,
                MAP_BYTES as u32,
                mapping_name.as_ptr(),
            )
        };
        let exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        let mapping = OwnedHandle::new(raw)?;
        ensure!(
            !exists,
            "shared-memory mapping name already exists; existing bytes preserved"
        );
        let address =
            unsafe { MapViewOfFile(mapping.0, FILE_MAP_READ | FILE_MAP_WRITE, 0, 0, MAP_BYTES) };
        ensure!(
            !address.Value.is_null(),
            "MapViewOfFile failed: {}",
            std::io::Error::last_os_error()
        );
        let view = View(address);
        let created = creation_time(unsafe { GetCurrentProcess() })?;
        drop(guard);
        Ok(Self {
            mutex,
            _mapping: mapping,
            view,
            created,
        })
    }
    pub(super) fn write(
        &mut self,
        payload: &[u8],
        sequence: u64,
        flags: u32,
    ) -> Result<Option<bool>> {
        ensure!(
            !payload.is_empty() && payload.len() <= MAP_BYTES - HEADER_BYTES,
            "shared-memory payload exceeds bounds"
        );
        let Some((_guard, abandoned)) = lock(&self.mutex, 0)? else {
            return Ok(None);
        };
        let (ticks, frequency) = qpc()?;
        let header = Header {
            abi_version: 1,
            payload_bytes: payload.len() as u32,
            publish_sequence: sequence,
            publisher_pid: std::process::id(),
            flags,
            published_qpc_ticks: ticks,
            qpc_frequency: frequency,
            process_creation_time: self.created,
        }
        .bytes();
        // The mapped view is at least MAP_BYTES, both copies are bounded, and the
        // named mutex excludes all conforming readers/writers during this commit.
        unsafe {
            let pointer = self.view.0.Value.cast::<u8>();
            ptr::copy_nonoverlapping(payload.as_ptr(), pointer.add(HEADER_BYTES), payload.len());
            ptr::copy_nonoverlapping(header.as_ptr(), pointer, HEADER_BYTES);
        }
        Ok(Some(abandoned))
    }
}

pub(super) fn read(config: &Config, wait_ms: u32, max_age_ms: u64) -> Result<Readout> {
    let name = wide(&config.mapping_name());
    let mutex_name = wide(&config.mutex_name());
    let mutex = OwnedHandle::new(unsafe {
        OpenMutexW(
            SYNCHRONIZATION_SYNCHRONIZE | MUTEX_MODIFY_STATE,
            0,
            mutex_name.as_ptr(),
        )
    })?;
    let mapping = OwnedHandle::new(unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) })?;
    let address = unsafe { MapViewOfFile(mapping.0, FILE_MAP_READ, 0, 0, MAP_BYTES) };
    ensure!(!address.Value.is_null(), "read-only MapViewOfFile failed");
    let view = View(address);
    let (guard, abandoned) =
        lock(&mutex, wait_ms)?.context("shared-memory reader mutex is busy")?;
    ensure!(
        !abandoned,
        "shared-memory mutex was abandoned; reject this observation"
    );
    let mut bytes = [0; HEADER_BYTES];
    unsafe {
        ptr::copy_nonoverlapping(view.0.Value.cast::<u8>(), bytes.as_mut_ptr(), HEADER_BYTES);
    }
    let header = Header::parse(&bytes)?;
    let mut payload = vec![0; header.payload_bytes as usize];
    unsafe {
        ptr::copy_nonoverlapping(
            view.0.Value.cast::<u8>().add(HEADER_BYTES),
            payload.as_mut_ptr(),
            payload.len(),
        );
    }
    drop(guard);
    let value: serde_json::Value = serde_json::from_slice(&payload)?;
    ensure!(
        value["format_version"] == 1
            && value["snapshot_sequence"].as_u64() == Some(header.publish_sequence)
            && value["sampling_active"].as_bool() == Some(header.flags & 1 != 0)
            && value["gateway_connected_last_observed"].as_bool() == Some(header.flags & 2 != 0),
        "shared-memory JSON/header mismatch"
    );
    let (ticks, frequency) = qpc()?;
    ensure!(
        frequency == header.qpc_frequency && ticks >= header.published_qpc_ticks,
        "inconsistent shared-memory QPC clock"
    );
    let age = ((u128::from(ticks - header.published_qpc_ticks) * 1_000_000) / u128::from(frequency))
        .min(u128::from(u64::MAX)) as u64;
    let process = unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
            0,
            header.publisher_pid,
        )
    };
    let alive = if process.is_null() {
        None
    } else {
        let process = OwnedHandle(process);
        Some(
            unsafe { WaitForSingleObject(process.0, 0) } == WAIT_TIMEOUT
                && creation_time(process.0)? == header.process_creation_time,
        )
    };
    Ok(Readout {
        format_version: 1,
        mapping_name: config.mapping_name(),
        header,
        publish_age_us: age,
        publish_age_exceeds_limit: age > max_age_ms.saturating_mul(1000),
        publisher_process_alive: alive,
        payload: value,
    })
}
