mod device;
mod ipa;
mod xcode;

use serde::{Deserialize, Serialize};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};
use std::{env, fs};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const LABEL: &str = "local.sideload";
const HOUR: u64 = 3600;
const REFRESH_BELOW: Duration = Duration::from_secs(72 * HOUR);
const WARN_BELOW: Duration = Duration::from_secs(24 * HOUR);
const USAGE: &str = "\
usage: sideload install <app.ipa> --team <id> [--bundle-id <id>] [--device <id>]
       sideload refresh [--force]
       sideload status
       sideload agent";

/// The app being kept alive, saved after every successful install.
#[derive(Serialize, Deserialize)]
struct State {
    name: String,
    ipa: PathBuf,
    bundle_id: String,
    team: String,
    device: String,
    expires: plist::Date,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct LaunchJob {
    label: &'static str,
    program_arguments: [String; 2],
    start_interval: u64,
    run_at_load: bool,
    standard_out_path: PathBuf,
    standard_error_path: PathBuf,
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("install") => install(&args[1..]),
        Some("refresh") => refresh(args.get(1).is_some_and(|a| a == "--force")),
        Some("status") => status(),
        Some("agent") => agent(),
        _ => Err(USAGE.into()),
    };
    if let Err(e) = result {
        eprintln!("{} error: {e}", now());
        std::process::exit(1);
    }
}

fn install(args: &[String]) -> Result<()> {
    let flag = |name: &str| {
        let i = args.iter().position(|a| a == name)?;
        args.get(i + 1).cloned()
    };
    let source = args.first().filter(|a| !a.starts_with('-'));
    let (Some(source), Some(team)) = (source, flag("--team")) else {
        return Err(USAGE.into());
    };
    let work = data_dir()?.join("work");
    let ipa = data_dir()?.join("app.ipa");
    if fs::canonicalize(source)? != fs::canonicalize(&ipa).unwrap_or_default() {
        fs::copy(source, &ipa)?;
    }
    let app = ipa::unpack(&ipa, &work)?;
    let bundle_id = match flag("--bundle-id") {
        Some(id) => id,
        None => format!("{}.{team}", ipa::bundle_id(&app)?),
    };
    let device = device::find(flag("--device").as_deref(), &work)?;
    let state = State {
        name: app.file_stem().unwrap_or_default().to_string_lossy().into(),
        ipa,
        bundle_id,
        team,
        device: device.id.clone(),
        expires: SystemTime::UNIX_EPOCH.into(),
    };
    deploy(state, &device, &app)
}

/// Run hourly by the launch agent. Does nothing until the profile is close to expiring.
fn refresh(force: bool) -> Result<()> {
    let state = load()?;
    let left = remaining(&state);
    if !force && left > REFRESH_BELOW {
        return Ok(());
    }
    let name = state.name.clone();
    let work = data_dir()?.join("work");
    let result = device::find(Some(&state.device), &work).and_then(|device| {
        let app = ipa::unpack(&state.ipa, &work)?;
        deploy(state, &device, &app)
    });
    if result.is_err() && left < WARN_BELOW {
        notify(&format!("{name} expires in {}. Connect the iPhone to this Mac by USB or Wi-Fi.", human(left)));
    }
    result
}

fn deploy(state: State, device: &device::Device, app: &Path) -> Result<()> {
    println!("{} signing {} as {} for {}", now(), state.name, state.bundle_id, device.name);
    let work = data_dir()?.join("work");
    let profile = xcode::fresh_profile(&work, &state.bundle_id, &state.team, &device.udid)?;
    ipa::resign(app, &state.bundle_id, &profile)?;
    device::install(device, app)?;
    let state = State { expires: profile.expires, ..state };
    plist::to_file_xml(data_dir()?.join("state.plist"), &state)?;
    let message = format!("{} valid until {}", state.name, state.expires.to_xml_format());
    println!("{} {message}", now());
    notify(&message);
    Ok(())
}

fn status() -> Result<()> {
    let state = load()?;
    let loaded = Command::new("launchctl").arg("print").arg(agent_target()?).output()?.status.success();
    println!("{} ({}) from {}", state.name, state.bundle_id, state.ipa.display());
    println!(
        "expires {} ({} left), refreshes below {}",
        state.expires.to_xml_format(),
        human(remaining(&state)),
        human(REFRESH_BELOW)
    );
    println!("agent {}", if loaded { "loaded" } else { "not loaded, run `sideload agent`" });
    Ok(())
}

fn agent() -> Result<()> {
    let exe = env::current_exe()?;
    let home = home()?;
    if ["Desktop", "Documents", "Downloads"].iter().any(|d| exe.starts_with(home.join(d))) {
        return Err("launchd needs a privacy prompt to run from this folder, use `cargo install --path .`".into());
    }
    let log = data_dir()?.join("refresh.log");
    let path = home.join(format!("Library/LaunchAgents/{LABEL}.plist"));
    let job = LaunchJob {
        label: LABEL,
        program_arguments: [exe.display().to_string(), "refresh".into()],
        start_interval: HOUR,
        run_at_load: true,
        standard_out_path: log.clone(),
        standard_error_path: log.clone(),
    };
    plist::to_file_xml(&path, &job)?;
    let _ = Command::new("launchctl").arg("bootout").arg(agent_target()?).output();
    run(Command::new("launchctl").arg("bootstrap").arg(format!("gui/{}", uid()?)).arg(&path))?;
    println!("loaded {}, logging to {}", path.display(), log.display());
    Ok(())
}

fn load() -> Result<State> {
    plist::from_file(data_dir()?.join("state.plist"))
        .map_err(|e| format!("nothing installed yet ({e}), run `sideload install` first").into())
}

fn run(command: &mut Command) -> Result<Vec<u8>> {
    let out = command.output()?;
    if out.status.success() {
        return Ok(out.stdout);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    Err(format!("{command:?} exited with {}\n{stderr}", out.status).into())
}

fn notify(message: &str) {
    let script = format!("display notification {message:?} with title \"sideload\"");
    let _ = Command::new("osascript").args(["-e", &script]).output();
}

fn remaining(state: &State) -> Duration {
    SystemTime::from(state.expires).duration_since(SystemTime::now()).unwrap_or_default()
}

fn human(d: Duration) -> String {
    let hours = d.as_secs() / HOUR;
    format!("{}d {}h", hours / 24, hours % 24)
}

fn now() -> String {
    plist::Date::from(SystemTime::now()).to_xml_format()
}

fn home() -> Result<PathBuf> {
    Ok(env::var("HOME")?.into())
}

fn uid() -> Result<u32> {
    Ok(fs::metadata(home()?)?.uid())
}

fn agent_target() -> Result<String> {
    Ok(format!("gui/{}/{LABEL}", uid()?))
}

fn data_dir() -> Result<PathBuf> {
    let dir = home()?.join("Library/Application Support/sideload");
    fs::create_dir_all(dir.join("work"))?;
    Ok(dir)
}
