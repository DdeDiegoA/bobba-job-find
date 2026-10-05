//! Free-space probe for the pre-migration snapshot check (AD-20).
//!
//! Kept tiny and platform-specific; the runner takes an injectable override so
//! the insufficient-disk branch is testable without filling a disk.

use std::io;
use std::path::Path;

#[cfg(unix)]
pub fn free_disk_bytes(path: &Path) -> io::Result<u64> {
    let stats = nix::sys::statvfs::statvfs(path).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(stats.blocks_available() as u64 * stats.fragment_size() as u64)
}

#[cfg(windows)]
pub fn free_disk_bytes(path: &Path) -> io::Result<u64> {
    use std::os::windows::ffi::OsStrExt as _;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    let mut available: u64 = 0;
    // SAFETY: `wide` is a NUL-terminated UTF-16 path; the outputs are valid for
    // the duration of the call.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(available)
}

#[cfg(not(any(unix, windows)))]
pub fn free_disk_bytes(_path: &Path) -> io::Result<u64> {
    Ok(u64::MAX)
}
