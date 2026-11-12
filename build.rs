//! Generates an embedded asset table from `web/dist` when present.

use std::{
	env,
	fs::{self, File},
	io::{self, Write},
	path::{Path, PathBuf},
};

fn main() -> io::Result<()> {
	println!("cargo:rerun-if-changed=web/dist");
	let out_dir = PathBuf::from(env::var_os("OUT_DIR").ok_or_else(|| {
		io::Error::new(io::ErrorKind::NotFound, "missing OUT_DIR environment variable")
	})?);
	let dest = out_dir.join("web_assets.rs");
	let mut out = File::create(dest)?;

	let dist = Path::new("web/dist");
	let mut entries = Vec::new();
	if dist.is_dir() {
		collect_files(dist, dist, &mut entries);
		entries.sort_by(|a, b| a.0.cmp(&b.0));
	}

	writeln!(out, "pub(super) static WEB_ASSETS: &[(&str, &str, &[u8])] = &[")?;
	for (route, abs_path) in entries {
		let mime = mime_for(&route);
		let escaped_path = abs_path.to_string_lossy().replace('\\', "/");
		writeln!(out, "\t({route:?}, {mime:?}, include_bytes!({escaped_path:?})),")?;
	}
	writeln!(out, "];")?;
	Ok(())
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
	let Ok(read_dir) = fs::read_dir(dir) else {
		return;
	};
	for entry in read_dir.flatten() {
		let Ok(ft) = entry.file_type() else {
			continue;
		};
		let path = entry.path();
		if ft.is_dir() {
			collect_files(root, &path, out);
		} else if ft.is_file()
			&& let Ok(rel) = path.strip_prefix(root)
			&& let Ok(abs) = fs::canonicalize(&path)
		{
			let clean_abs = abs
				.to_str()
				.and_then(|s| s.strip_prefix(r"\\?\"))
				.map_or_else(|| abs.clone(), PathBuf::from);
			let route = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
			out.push((route, clean_abs));
		}
	}
}

fn mime_for(route: &str) -> &'static str {
	match route.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
		"html" => "text/html; charset=utf-8",
		"js" | "mjs" => "text/javascript; charset=utf-8",
		"css" => "text/css; charset=utf-8",
		"json" | "map" => "application/json; charset=utf-8",
		"svg" => "image/svg+xml",
		"woff2" => "font/woff2",
		"woff" => "font/woff",
		"png" => "image/png",
		"webp" => "image/webp",
		"ico" => "image/x-icon",
		"txt" => "text/plain; charset=utf-8",
		_ => "application/octet-stream",
	}
}
