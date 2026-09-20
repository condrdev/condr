//! `condr device …`: the remote Devices this machine has saved, and how `--device`
//! reaches one. Every call connects on its own, like a call to the local socket.

use std::thread;

use super::*;
use condr_server::load_saved_servers;

#[derive(Subcommand)]
pub(crate) enum DeviceCommand {
    /// List the saved Devices and whether each answers right now
    List,
}

#[derive(Serialize)]
struct DeviceInfo {
    name: String,
    /// The address without key material, as the GUI shows it.
    address: String,
    reachable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_count: Option<usize>,
}

/// `[[client.servers]]` from the shared `config.toml`.
pub(super) fn saved_devices() -> Result<Vec<SavedServer>, CliError> {
    let path = condr_core::config_path().ok_or_else(|| {
        CliError::new("no_config_directory", "cannot locate the config directory")
    })?;
    load_saved_servers(&path).map_err(|error| {
        CliError::new(
            "config_invalid",
            format!("failed to load {}: {error}", path.display()),
        )
    })
}

/// The Endpoint of the saved Device called `name`. A TCP Device speaks with this
/// machine's device key, the one the GUI paired with.
pub(super) fn device_endpoint(devices: &[SavedServer], name: &str) -> Result<Endpoint, CliError> {
    let device = devices
        .iter()
        .find(|device| device.name == name)
        .ok_or_else(|| {
            let known = devices
                .iter()
                .map(|device| device.name.as_str())
                .collect::<Vec<_>>();
            CliError::new(
                "device_not_found",
                if known.is_empty() {
                    format!("no device named {name:?}; none are saved in config.toml")
                } else {
                    format!(
                        "no device named {name:?}; saved devices: {}",
                        known.join(", ")
                    )
                },
            )
        })?;
    let key = device
        .is_tcp()
        .then(|| {
            let directory = condr_core::config_directory().ok_or_else(|| {
                CliError::new("no_config_directory", "cannot locate the config directory")
            })?;
            condr_server::noise::load_device_key(&directory)
                .map_err(|error| CliError::new("device_key", error.to_string()))
        })
        .transpose()?;
    device
        .endpoint(key.as_ref())
        .map_err(|error| CliError::new("device_address_invalid", error.to_string()))
}

/// Connects to every Device at once; a slow or dead one must not hold up the rest.
fn probe_all(devices: &[SavedServer]) -> Vec<DeviceInfo> {
    thread::scope(|scope| {
        let probes = devices
            .iter()
            .map(|device| scope.spawn(move || probe(devices, device)))
            .collect::<Vec<_>>();
        probes
            .into_iter()
            .map(|probe| probe.join().expect("device probe panicked"))
            .collect()
    })
}

fn probe(devices: &[SavedServer], device: &SavedServer) -> DeviceInfo {
    let mut info = DeviceInfo {
        name: device.name.clone(),
        address: device.address.clone(),
        reachable: false,
        error: None,
        workspace_count: None,
    };
    match device_endpoint(devices, &device.name).and_then(|endpoint| {
        info.address = endpoint.to_string();
        connect_to(&endpoint)
    }) {
        Ok(client) => match client.session() {
            Ok(session) => {
                info.reachable = true;
                info.workspace_count = Some(session.workspaces().len());
            }
            Err(error) => info.error = Some(error.to_string()),
        },
        Err(error) => info.error = Some(error.message),
    }
    info
}

/// One Device's name and its Workspaces, or why it could not be listed.
pub(super) type DeviceListing = (
    String,
    Result<Vec<super::workspace::WorkspaceInfo>, CliError>,
);

/// Every saved Device's Workspaces, connecting to all of them at once. The command's
/// own target (`base`, when `--device` chose one) is already listed, so it is skipped.
pub(super) fn list_workspaces_everywhere(
    base: Option<&str>,
) -> Result<Vec<DeviceListing>, CliError> {
    let devices = &saved_devices()?;
    Ok(thread::scope(|scope| {
        let listings = devices
            .iter()
            .filter(|device| base != Some(device.name.as_str()))
            .map(|device| {
                scope.spawn(move || {
                    let listing = device_endpoint(devices, &device.name)
                        .and_then(|endpoint| connect_to(&endpoint))
                        .and_then(|client| Ok(client.session()?))
                        .map(|session| {
                            session
                                .workspaces()
                                .iter()
                                .map(super::workspace::workspace_info)
                                .collect()
                        });
                    (device.name.clone(), listing)
                })
            })
            .collect::<Vec<_>>();
        listings
            .into_iter()
            .map(|listing| listing.join().expect("device listing panicked"))
            .collect()
    }))
}

pub(crate) fn run_device(_device: Option<&str>, command: DeviceCommand) -> i32 {
    let result = match command {
        DeviceCommand::List => {
            saved_devices().map(|devices| json!({ "devices": probe_all(&devices) }))
        }
    };
    match result {
        Ok(value) => {
            println!("{value}");
            0
        }
        Err(error) => {
            eprintln!("{}", json!({ "error": error }));
            1
        }
    }
}
