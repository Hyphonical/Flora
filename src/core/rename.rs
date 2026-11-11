//! Rename a file or folder without replacing an existing destination.

use std::{io, path::Path};

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) fn no_replace(from: &Path, to: &Path) -> io::Result<()> {
	use std::{ffi::CString, os::unix::ffi::OsStrExt};
	let from = CString::new(from.as_os_str().as_bytes())
		.map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid source path"))?;
	let to = CString::new(to.as_os_str().as_bytes())
		.map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid destination path"))?;
	// The NUL-terminated paths stay valid for the system call.
	#[cfg(target_os = "linux")]
	let result = unsafe {
		libc::renameat2(
			libc::AT_FDCWD,
			from.as_ptr(),
			libc::AT_FDCWD,
			to.as_ptr(),
			libc::RENAME_NOREPLACE,
		)
	};
	#[cfg(target_os = "macos")]
	let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
	if result == 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}

#[cfg(windows)]
pub(crate) fn no_replace(from: &Path, to: &Path) -> io::Result<()> {
	use std::os::windows::ffi::OsStrExt;
	let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
	let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
	// MoveFileW fails when the destination already exists.
	let result =
		unsafe { windows_sys::Win32::Storage::FileSystem::MoveFileW(from.as_ptr(), to.as_ptr()) };
	if result != 0 { Ok(()) } else { Err(io::Error::last_os_error()) }
}
