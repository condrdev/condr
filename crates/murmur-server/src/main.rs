use std::net::SocketAddr;
use std::path::PathBuf;

use murmur_server::{Endpoint, ServerConfig, run};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--help" | "-h"))
    {
        println!("murmur-server [--endpoint PATH | --listen ADDR] [--snapshot PATH] [--stop]");
        return;
    }
    let (endpoint, snapshot_path, stop) = match parse_args(&args) {
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
    let config = snapshot_path.map_or_else(
        || ServerConfig::new(endpoint.clone()),
        |path| ServerConfig::new(endpoint.clone()).with_snapshot_path(path),
    );
    if let Err(error) = run(config) {
        eprintln!("murmur-server: {error}");
        std::process::exit(1);
    }
}

fn parse_args(args: &[String]) -> Result<(Endpoint, Option<PathBuf>, bool), String> {
    let mut endpoint = None;
    let mut snapshot_path = None;
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
            "--snapshot" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--snapshot requires a path".to_string())?;
                snapshot_path = Some(PathBuf::from(value));
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
        snapshot_path,
        stop,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_path_can_be_overridden_for_a_local_server() {
        let args = [
            "murmur-server".into(),
            "--endpoint".into(),
            "test.sock".into(),
            "--snapshot".into(),
            "test.snapshot".into(),
        ];

        let (endpoint, snapshot, stop) = parse_args(&args).unwrap();

        assert_eq!(endpoint, Endpoint::local("test.sock"));
        assert_eq!(snapshot, Some(PathBuf::from("test.snapshot")));
        assert!(!stop);
    }

    #[test]
    fn snapshot_flag_requires_a_path() {
        let args = ["murmur-server".into(), "--snapshot".into()];

        assert_eq!(parse_args(&args).unwrap_err(), "--snapshot requires a path");
    }
}
