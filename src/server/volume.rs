//! Identify the filesystem volume used for the configured disk ceiling.

use anyhow::Result;
use std::path::Path;

#[cfg(unix)]
pub(super) fn same_volume(first: &Path, second: &Path) -> Result<bool> {
	use std::os::unix::fs::MetadataExt;
	Ok(std::fs::metadata(first)?.dev() == std::fs::metadata(second)?.dev())
}

#[cfg(windows)]
pub(super) fn same_volume(first: &Path, second: &Path) -> Result<bool> {
	use std::os::windows::ffi::OsStrExt;
	use windows_sys::Win32::Storage::FileSystem::{
		GetVolumeNameForVolumeMountPointW, GetVolumePathNameW,
	};

	fn volume(path: &Path) -> Result<Vec<u16>> {
		let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
		let mut mount = vec![0u16; 32768];
		let mut name = vec![0u16; 64];
		// These APIs write NUL-terminated UTF-16 into the supplied buffers.
		unsafe {
			if GetVolumePathNameW(path.as_ptr(), mount.as_mut_ptr(), mount.len() as u32) == 0 {
				return Err(std::io::Error::last_os_error().into());
			}
			if GetVolumeNameForVolumeMountPointW(
				mount.as_ptr(),
				name.as_mut_ptr(),
				name.len() as u32,
			) == 0
			{
				return Err(std::io::Error::last_os_error().into());
			}
		}
		name.truncate(name.iter().position(|&unit| unit == 0).unwrap_or(name.len()));
		Ok(name)
	}
	Ok(volume(first)? == volume(second)?)
}
