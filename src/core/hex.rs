//! Encode bytes as lowercase hexadecimal for tokens and metadata.

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
