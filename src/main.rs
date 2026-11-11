//! Flora's single server and command-line entry point.

mod cli;
mod core;
mod server;

use anyhow::Result;
use core::config::{Command, Config, OutputFlags};

#[tokio::main]
async fn main() {
	let early_flags = OutputFlags::from_process();
	let config = match Config::parse() {
		Ok(config) => config,
		Err(error) => {
			cli::error(early_flags, None, &format!("{error:#}"));
			std::process::exit(1);
		}
	};
	if let Err(error) = run(&config).await {
		cli::error(
			config.output_flags(),
			Some(config.action.name()),
			&format!("{error:#}"),
		);
		std::process::exit(1);
	}
}

async fn run(config: &Config) -> Result<()> {
	match &config.action {
		Command::Version => {
			cli::version();
			Ok(())
		}
		Command::Help(command) => {
			cli::help(config.output_flags(), command.as_deref());
			Ok(())
		}
		Command::Serve(serve) => {
			core::logging::init(&config.global.log, config.global.quiet, true)?;
			server::run(serve).await
		}
		_ => {
			core::logging::init(&config.global.log, config.global.quiet, false)?;
			cli::run(config).await
		}
	}
}
