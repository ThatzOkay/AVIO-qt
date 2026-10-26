use std::{collections::HashSet, path::PathBuf, process::Stdio, sync::Mutex};

use regex::Regex;
use tokio::{io::AsyncWriteExt, process::Command};

use crate::paths::assets_dir;

type BoxError = Box<dyn std::error::Error + Send + Sync>;

const RULE_FILE: &str = "/etc/udev/rules.d/99-AVIO.rules";

const TEMPLATE_FILENAME: &str = "99-AVIO.rules.template";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleStatus {
    Current,
    Missing,
    Outdated,
}

fn template_path() -> PathBuf {
    assets_dir().join("linux").join(TEMPLATE_FILENAME)
}

static CACHED_PHONE_VENDOR_IDS: Mutex<Option<HashSet<u16>>> = Mutex::new(None);

fn load_template() -> Result<String, BoxError> {
    Ok(std::fs::read_to_string(template_path())?)
}

// Android vendor allowlist parsed from the udev template.
// Lines that also match an idProduct (dongle) are skipped.
pub fn phone_vendor_ids_from_udev_template() -> Result<HashSet<u16>, BoxError> {
    {
        let cache = CACHED_PHONE_VENDOR_IDS.lock().unwrap();
        if let Some(ids) = &*cache {
            return Ok(ids.clone());
        }
    }

    let regex = Regex::new(r#"ATTR\{idVendor\}==\"([0-9a-fA-F]{4})\""#)?;
    let mut ids: HashSet<u16> = HashSet::new();

    let template = load_template()?;
    for line in template.lines() {
        if line.contains("ATTR{idProduct}") {
            continue;
        }
        if let Some(captures) = regex.captures(line) {
            if let Some(vendor_id_str) = captures.get(1) {
                if let Ok(vendor_id) = u16::from_str_radix(vendor_id_str.as_str(), 16) {
                    ids.insert(vendor_id);
                }
            }
        }
    }

    *CACHED_PHONE_VENDOR_IDS.lock().unwrap() = Some(ids.clone());

    Ok(ids)
}

fn template_marker(template: &str) -> String {
    let re = Regex::new(r"(?m)^# AVIO-RULE-VERSION=\d+$").unwrap();
    re.find(template)
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "# AVIO-RULE-VERSION=0".to_string())
}

fn resolve_username() -> Result<String, BoxError> {
    if let Ok(username) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
        return Ok(username);
    }

    let output = std::process::Command::new("id").arg("-un").output()?;
    if !output.status.success() {
        return Err(Box::new(std::io::Error::other("could not resolve the current user name")));
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

fn build_rule_content() -> Result<String, BoxError> {
    let template = load_template()?;
    let username = resolve_username()?;
    Ok(template.replace("__USERNAME__", &username))
}

pub fn udev_rule_exists() -> bool {
    std::path::Path::new(RULE_FILE).exists()
}

fn udev_rule_is_current() -> Result<bool, BoxError> {
    if !udev_rule_exists() {
        return Ok(false);
    }

    let current_content = std::fs::read_to_string(RULE_FILE)?;
    let expected_content = template_marker(&load_template()?);
    Ok(current_content.contains(&expected_content))
}

pub fn rule_file() -> &'static str {
    RULE_FILE
}

// Whether the installed rule is there and matches the template's version. Always Current off Linux,
// where there's nothing to install.
pub fn rule_status() -> RuleStatus {
    if std::env::consts::OS != "linux" {
        return RuleStatus::Current;
    }

    let exists = udev_rule_exists();
    let is_current = udev_rule_is_current().unwrap_or_else(|e| {
        eprintln!("Failed to check udev rule version: {:?}", e);
        false
    });

    match (exists, is_current) {
        (true, true) => RuleStatus::Current,
        (true, false) => RuleStatus::Outdated,
        (false, _) => RuleStatus::Missing,
    }
}

// Tells the nested avio-compositor (if we're running under one) to re-exec itself once our
// toplevel goes away, instead of tearing down the whole kiosk session (cage/login/getty) — see
// avio-compositor.c's "restart" control-socket command and `full_restart` flag.
#[cfg(target_os = "linux")]
fn request_compositor_restart() {
    let Ok(path) = std::env::var("AVIO_COMPOSITOR_CTRL") else {
        return;
    };
    use std::io::Write;
    if let Ok(mut sock) = std::os::unix::net::UnixStream::connect(&path) {
        let _ = sock.write_all(b"restart\n");
    }
}

#[cfg(not(target_os = "linux"))]
fn request_compositor_restart() {}

// Relaunches the app so the new rule applies to already-plugged-in devices, then exits
pub fn restart_app() {
    request_compositor_restart();
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn() {
                eprintln!("Failed to relaunch: {:?}", e);
            }
        }
        Err(e) => eprintln!("Failed to resolve the executable path: {:?}", e),
    }
    std::process::exit(0);
}

pub fn pkexec_available() -> bool {
    std::process::Command::new("which")
        .arg("pkexec")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

// Already root (the usual case on an embedded image): no need to ask polkit
#[cfg(target_os = "linux")]
fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions and cannot fail
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(target_os = "linux"))]
fn is_root() -> bool {
    false
}

// Whether `install_rule` can work here at all: as root directly, otherwise through pkexec.
// Without either (a Buildroot image running as a plain user and no polkit) the rule has to be
// part of the image, so there's nothing to offer the user.
pub fn can_install() -> bool {
    is_root() || pkexec_available()
}

pub async fn install_rule() -> Result<(), BoxError> {
    let content = build_rule_content()?;

    if is_root() {
        std::fs::write(RULE_FILE, content)?;
        run_udevadm(&["control", "--reload-rules"]).await?;
        run_udevadm(&["trigger"]).await?;
        return Ok(());
    }

    if !pkexec_available() {
        return Err(Box::new(std::io::Error::other(
            "pkexec is not available. Cannot install udev rule.",
        )));
    }

    let script = format!(
        "cat > {RULE_FILE} && udevadm control --reload-rules && udevadm trigger"
    );

    // The rule goes in over stdin so quotes or $ in the template can't be mangled by the shell
    let mut proc = Command::new("pkexec")
        .arg("bash")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = proc.stdin.take() {
        stdin.write_all(content.as_bytes()).await?;
    }

    let code = proc.wait().await?.code().unwrap_or(-1);
    if code != 0 {
        return Err(Box::new(std::io::Error::other(format!(
            "pkexec command failed with exit code {}",
            code
        ))));
    }

    Ok(())
}

async fn run_udevadm(args: &[&str]) -> Result<(), BoxError> {
    let status = Command::new("udevadm").args(args).status().await?;
    if !status.success() {
        return Err(Box::new(std::io::Error::other(format!(
            "udevadm {} failed with {status}",
            args.join(" ")
        ))));
    }
    Ok(())
}
