use crate::{Result, home, run};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// A provisioning profile issued by Xcode, plus what is needed to sign with it.
pub struct Profile {
    pub file: PathBuf,
    pub expires: plist::Date,
    pub identity: String,
    pub entitlements: PathBuf,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Decoded {
    name: String,
    expiration_date: plist::Date,
    team_identifier: Vec<String>,
}

/// Builds an empty app with `bundle_id` so Xcode issues a profile through its own Apple ID login.
/// Xcode reuses a cached profile until it expires, so the cached copy is deleted first.
pub fn fresh_profile(work: &Path, bundle_id: &str, team: &str, udid: &str) -> Result<Profile> {
    forget_cached(bundle_id, team)?;

    let project = work.join("stub/Stub.xcodeproj");
    fs::create_dir_all(&project)?;
    fs::create_dir_all(work.join("stub/Src"))?;
    fs::write(project.join("project.pbxproj"), include_str!("stub.pbxproj"))?;
    fs::write(work.join("stub/Src/main.swift"), "print(\"stub\")\n")?;
    let derived = work.join("derived");
    let _ = fs::remove_dir_all(&derived);
    run(Command::new("xcodebuild")
        .arg("-project")
        .arg(&project)
        .args(["-scheme", "Stub", "-configuration", "Release", "-quiet"])
        .args(["-allowProvisioningUpdates", "-allowProvisioningDeviceRegistration"])
        .args(["-destination", &format!("id={udid}")])
        .arg("-derivedDataPath")
        .arg(&derived)
        .arg(format!("PRODUCT_BUNDLE_IDENTIFIER={bundle_id}"))
        .arg(format!("DEVELOPMENT_TEAM={team}"))
        .arg("build"))?;

    let app = derived.join("Build/Products/Release-iphoneos/Stub.app");
    let certs = work.join("cert");
    run(Command::new("codesign").arg("-d").arg(format!("--extract-certificates={}", certs.display())).arg(&app))?;
    let entitlements = work.join("entitlements.plist");
    let signed = run(Command::new("codesign").args(["-d", "--entitlements", "-", "--xml"]).arg(&app))?;
    fs::write(&entitlements, signed)?;

    let file = app.join("embedded.mobileprovision");
    let leaf = fs::read(work.join("cert0"))?;
    Ok(Profile {
        expires: decode(&file)?.expiration_date,
        file,
        identity: sha1_smol::Sha1::from(leaf).digest().to_string().to_uppercase(),
        entitlements,
    })
}

fn forget_cached(bundle_id: &str, team: &str) -> Result<()> {
    let name = format!("iOS Team Provisioning Profile: {bundle_id}");
    let dir = home()?.join("Library/Developer/Xcode/UserData/Provisioning Profiles");
    for entry in fs::read_dir(dir).into_iter().flatten() {
        let path = entry?.path();
        let matches = |p: Decoded| p.name == name && p.team_identifier.iter().any(|t| t == team);
        if path.extension().is_some_and(|e| e == "mobileprovision") && decode(&path).is_ok_and(matches) {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn decode(profile: &Path) -> Result<Decoded> {
    let xml = run(Command::new("security").args(["cms", "-D", "-i"]).arg(profile))?;
    Ok(plist::from_bytes(&xml)?)
}
