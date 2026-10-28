use crate::{Result, run};
use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::process::Command;

pub struct Device {
    pub id: String,
    pub udid: String,
    pub name: String,
}

#[derive(Deserialize)]
struct Listing {
    result: Devices,
}

#[derive(Deserialize)]
struct Devices {
    devices: Vec<Raw>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Raw {
    identifier: String,
    hardware_properties: Hardware,
    device_properties: Properties,
    connection_properties: Connection,
}

#[derive(Deserialize)]
struct Hardware {
    udid: String,
}

#[derive(Deserialize)]
struct Properties {
    name: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    transport_type: Option<String>,
}

/// The paired device matching `wanted` (`CoreDevice` id, UDID or name), or the first one. Fails if it is unreachable.
pub fn find(wanted: Option<&str>, work: &Path) -> Result<Device> {
    let json = work.join("devices.json");
    run(Command::new("xcrun").args(["devicectl", "list", "devices", "--json-output"]).arg(&json))?;
    let listing: Listing = serde_json::from_slice(&fs::read(&json)?)?;
    let raw = listing
        .result
        .devices
        .into_iter()
        .find(|d| {
            let ids = [&d.identifier, &d.hardware_properties.udid, &d.device_properties.name];
            wanted.is_none_or(|w| ids.iter().any(|id| *id == w))
        })
        .ok_or_else(|| format!("no paired device matching {}", wanted.unwrap_or("any")))?;
    let name = raw.device_properties.name;
    if raw.connection_properties.transport_type.is_none() {
        return Err(format!("{name} is not reachable, connect it by USB or the same Wi-Fi").into());
    }
    Ok(Device { id: raw.identifier, udid: raw.hardware_properties.udid, name })
}

pub fn install(device: &Device, app: &Path) -> Result<()> {
    run(Command::new("xcrun").args(["devicectl", "device", "install", "app", "--device", &device.id]).arg(app))?;
    Ok(())
}
