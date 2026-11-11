//! Hexadecimal encoding, validation, and streaming XXH3-128 file hashing.

use std::{io::Read, path::Path};
use tokio::io::AsyncReadExt;
use xxhash_rust::xxh3::Xxh3;

pub(crate) fn encode(bytes: impl AsRef<[u8]>) -> String {
	const DIGITS: &[u8; 16] = b"0123456789abcdef";
	let bytes = bytes.as_ref();
	let mut encoded = String::with_capacity(bytes.len() * 2);
	for &byte in bytes {
		encoded.push(char::from(DIGITS[usize::from(byte >> 4)]));
		encoded.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
	}
	encoded
}

pub(crate) fn u128(value: u128) -> String {
	encode(value.to_be_bytes())
}

pub(crate) fn is_hex_128(value: &str) -> bool {
	value.len() == 32 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

pub(crate) fn hash_file_sync(path: &Path) -> std::io::Result<String> {
	let mut file = std::fs::File::open(path)?;
	let mut hash = Xxh3::new();
	let mut buffer = [0u8; 64 * 1024];
	loop {
		let n = file.read(&mut buffer)?;
		if n == 0 {
			break;
		}
		hash.update(&buffer[..n]);
	}
	Ok(u128(hash.digest128()))
}

pub(crate) async fn hash_file_async(path: &Path) -> std::io::Result<String> {
	let mut file = tokio::fs::File::open(path).await?;
	let mut hash = Xxh3::new();
	let mut buffer = vec![0u8; 64 * 1024];
	loop {
		let n = file.read(&mut buffer).await?;
		if n == 0 {
			break;
		}
		hash.update(&buffer[..n]);
	}
	Ok(u128(hash.digest128()))
}
