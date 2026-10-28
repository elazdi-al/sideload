use crate::xcode::Profile;
use crate::{Result, run};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Extracts `ipa` into a clean `work/app` and returns the `.app` bundle inside `Payload`.
pub fn unpack(ipa: &Path, work: &Path) -> Result<PathBuf> {
    let out = work.join("app");
    let _ = fs::remove_dir_all(&out);
    run(Command::new("ditto").args(["-x", "-k"]).arg(ipa).arg(&out))?;
    fs::read_dir(out.join("Payload"))?
        .filter_map(|e| Some(e.ok()?.path()))
        .find(|p| p.extension().is_some_and(|e| e == "app"))
        .ok_or_else(|| format!("{} has no Payload/*.app", ipa.display()).into())
}

pub fn bundle_id(app: &Path) -> Result<String> {
    let info: plist::Dictionary = plist::from_file(app.join("Info.plist"))?;
    let id = info.get("CFBundleIdentifier").and_then(plist::Value::as_string);
    Ok(id.ok_or("Info.plist has no CFBundleIdentifier")?.into())
}

pub fn resign(app: &Path, bundle_id: &str, profile: &Profile) -> Result<()> {
    // Each extension would need its own App ID, and free teams get 10 per week.
    for dir in ["PlugIns", "Extensions", "Watch", "com.apple.WatchPlaceholder"] {
        let _ = fs::remove_dir_all(app.join(dir));
    }
    let info_path = app.join("Info.plist");
    let mut info: plist::Dictionary = plist::from_file(&info_path)?;
    info.insert("CFBundleIdentifier".into(), bundle_id.into());
    info.remove("UISupportedDevices");
    plist::to_file_binary(&info_path, &info)?;
    fs::copy(&profile.file, app.join("embedded.mobileprovision"))?;

    let mut nested = Vec::new();
    nested_code(app, &mut nested)?;
    for code in &nested {
        run(Command::new("codesign").args(["-f", "-s", &profile.identity]).arg(code))?;
    }
    run(Command::new("codesign")
        .args(["-f", "-s", &profile.identity, "--entitlements"])
        .arg(&profile.entitlements)
        .arg(app))?;
    run(Command::new("codesign").args(["--verify", "--deep", "--strict"]).arg(app))?;
    Ok(())
}

/// Frameworks and dylibs under `dir`, innermost first so each container is signed after its contents.
fn nested_code(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            nested_code(&path, out)?;
        }
        if path.extension().is_some_and(|e| e == "framework" || e == "dylib") {
            out.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_code_signs_inner_code_first() {
        let root = std::env::temp_dir().join(format!("sideload-test-{}", std::process::id()));
        fs::create_dir_all(root.join("Frameworks/A.framework/Frameworks/B.framework")).unwrap();
        fs::write(root.join("Frameworks/lib.dylib"), "").unwrap();
        fs::write(root.join("Info.plist"), "").unwrap();

        let mut found = Vec::new();
        nested_code(&root, &mut found).unwrap();
        fs::remove_dir_all(&root).unwrap();

        let rel: Vec<_> = found.iter().map(|p| p.strip_prefix(&root).unwrap().to_str().unwrap()).collect();
        let pos = |name| rel.iter().position(|r| *r == name).unwrap();
        assert_eq!(rel.len(), 3, "{rel:?}");
        assert!(pos("Frameworks/A.framework/Frameworks/B.framework") < pos("Frameworks/A.framework"));
        assert!(rel.contains(&"Frameworks/lib.dylib"));
    }
}
