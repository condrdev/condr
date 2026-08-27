use std::net::SocketAddr;
use std::path::PathBuf;

use murmur_server::{Endpoint, ServerConfig, run};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        println!("murmur-server [--endpoint PATH | --listen ADDR] [--stop]");
        return;
    }
    let (endpoint, stop) = match parse_args(&args) {
        Ok(result) => result,
        Err(error) => {
            eprintln!("murmur-server: {error}");
            std::process::exit(2);
        }
    };
    if stop {
        if let Err(error) = murmur_server::stop_server(&endpoint) {
            eprintln!("murmur-server: {error}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = run(ServerConfig { endpoint }) {
        eprintln!("murmur-server: {error}");
        std::process::exit(1);
    }
}

fn parse_args(args: &[String]) -> Result<(Endpoint, bool), String> {
    let mut endpoint = None;
    let mut stop = false;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--endpoint" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--endpoint requires a path".to_string())?;
                endpoint = Some(Endpoint::local(PathBuf::from(value)));
                index += 2;
            }
            "--listen" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--listen requires an address".to_string())?;
                let address: SocketAddr = value
                    .parse()
                    .map_err(|error| format!("invalid listen address {value}: {error}"))?;
                endpoint = Some(Endpoint::tcp(address));
                index += 2;
            }
            "--stop" => {
                stop = true;
                index += 1;
            }
            unknown => return Err(format!("unknown argument {unknown}")),
        }
    }
    Ok((
        endpoint.unwrap_or_else(|| ServerConfig::default().endpoint),
        stop,
    ))
}
