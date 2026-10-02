use std::io::ErrorKind;
use std::process::ExitCode;

use clap::Parser;
use maxwells_demon_detector::{run, Cli};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        // Output piped into something like `head` that stopped reading: not worth reporting.
        Err(e)
            if e.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == ErrorKind::BrokenPipe) =>
        {
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("mdd: {e}");
            ExitCode::FAILURE
        }
    }
}
