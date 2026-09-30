//! A file's id on its volume: the third part of the unchanged check (size,
//! mtime, file id; ROADMAP 1.1, 1aC-1). A rename or a move within the
//! volume keeps it; a copy gets a new one.
//!
//! Getting it takes a handle, opened with **access 0**: a query-only handle
//! that can't read or write the file's data, so it never pulls down a
//! OneDrive placeholder or changes a byte. It's opened with
//! `FILE_FLAG_OPEN_REPARSE_POINT`, so a link or placeholder is asked about
//! itself and never followed or recalled, and shares everything, so it
//! never gets in the way of another program (rekordbox included).

use std::ffi::c_void;
use std::io;
use std::iter;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FileIdInfo, GetFileInformationByHandleEx, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_ID_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, OPEN_EXISTING,
};

/// A query-only handle, closed on drop.
pub(super) struct QueryHandle(HANDLE);

impl QueryHandle {
    /// Opens `path` (a `\\?\` path) with access 0: no read, no write.
    pub(super) fn open(path: &Path) -> io::Result<QueryHandle> {
        let wide: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(iter::once(0))
            .collect();
        // Access 0 (query only), OPEN_EXISTING (never creates), no write or
        // delete flags. BACKUP_SEMANTICS lets the same call open a folder.
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        Ok(QueryHandle(handle))
    }

    #[cfg(test)]
    pub(super) fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for QueryHandle {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// The file's volume serial and 128-bit file id, as
/// `<serial, 16 hex digits>-<id, 32 hex digits>`. Text, because it doesn't
/// fit a 64-bit integer. Fails on filesystems that have no stable file id
/// to report (some FAT and network drives); the caller stores none then.
pub(crate) fn file_id(path: &Path) -> io::Result<String> {
    let handle = QueryHandle::open(path)?;
    let mut info = FILE_ID_INFO::default();
    let ok = unsafe {
        GetFileInformationByHandleEx(
            handle.0,
            FileIdInfo,
            (&mut info as *mut FILE_ID_INFO).cast::<c_void>(),
            mem::size_of::<FILE_ID_INFO>() as u32,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut id = format!("{:016x}-", info.VolumeSerialNumber);
    // Most significant byte first, so the text reads as one 128-bit number.
    for byte in info.FileId.Identifier.iter().rev() {
        id.push_str(&format!("{byte:02x}"));
    }
    Ok(id)
}
