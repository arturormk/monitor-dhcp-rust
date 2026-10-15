use monitor_dhcp::config::Config;
use monitor_dhcp::server;
use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

const DEFAULT_CONFIG: &str = "/etc/monitor-dhcp/config.toml";
const VERSION: &str = env!("MONITOR_DHCP_VERSION");

fn main() -> ExitCode {
    match real_main() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("monitor-dhcp: {error}");
            ExitCode::FAILURE
        }
    }
}

fn real_main() -> Result<(), String> {
    let args = parse_args(env::args().skip(1))?;
    if args.help {
        print_help();
        return Ok(());
    }
    if args.version {
        println!("monitor-dhcp v{VERSION}");
        return Ok(());
    }

    let config = Config::load(&args.config)?;
    if args.check {
        println!("configuration is valid (network interface not checked)");
        return Ok(());
    }

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format_timestamp_secs()
        .init();
    server::run(&args.config, config)
}

struct Args {
    config: PathBuf,
    check: bool,
    help: bool,
    version: bool,
}

fn parse_args(arguments: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        config: PathBuf::from(DEFAULT_CONFIG),
        check: false,
        help: false,
        version: false,
    };
    let mut arguments = arguments.peekable();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--config" => {
                args.config = arguments
                    .next()
                    .map(PathBuf::from)
                    .ok_or_else(|| "--config requires a path".to_string())?;
            }
            "--check" => args.check = true,
            "-h" | "--help" => args.help = true,
            "-V" | "--version" => args.version = true,
            _ => return Err(format!("unknown argument '{argument}' (try --help)")),
        }
    }
    Ok(args)
}

fn print_help() {
    println!(
        "monitor-dhcp {}\n\nUSAGE:\n    monitor-dhcp [--config PATH] [--check]\n\nOPTIONS:\n    --config PATH  Configuration file (default: {DEFAULT_CONFIG})\n    --check        Validate syntax and static semantics without opening a socket\n    -h, --help     Print help\n    -V, --version  Print version\n\nLOGGING:\n    Set RUST_LOG to error, warn, info, or debug (default: info).",
        VERSION
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_cli() {
        let args = parse_args(
            ["--config", "test.toml", "--check"]
                .into_iter()
                .map(str::to_string),
        )
        .unwrap();
        assert_eq!(args.config, PathBuf::from("test.toml"));
        assert!(args.check);
    }

    #[test]
    fn rejects_unknown_cli_argument() {
        assert!(parse_args(["--wat"].into_iter().map(str::to_string)).is_err());
    }
}
