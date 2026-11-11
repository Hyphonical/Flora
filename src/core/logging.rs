//! Logging initialization shared by the CLI and server.

use anyhow::{Context, Result};
use tracing_subscriber::{
	filter::{LevelFilter, Targets},
	prelude::*,
};

pub(crate) fn init(log_level: &str, quiet: bool, serve: bool) -> Result<()> {
	let mut level: LevelFilter = log_level.parse().context("invalid --log level")?;
	if quiet && !serve {
		level = LevelFilter::ERROR;
	}
	let other = if serve { LevelFilter::WARN.min(level) } else { LevelFilter::ERROR.min(level) };
	let filter = Targets::new().with_default(other).with_target("flora", level);
	tracing_subscriber::registry()
		.with(filter)
		.with(
			tracing_subscriber::fmt::layer()
				.with_target(false)
				.with_ansi(false)
				.with_writer(std::io::stderr),
		)
		.try_init()
		.map_err(|error| anyhow::anyhow!(error.to_string()))?;
	Ok(())
}
