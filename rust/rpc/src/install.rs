//! `lumisound-discord-rpc install` / `uninstall`: signs in (same as `login`)
//! and registers the daemon to start with the user's session. Replaces the
//! old install.sh / install-macos.sh / install-windows.ps1 scripts.
//!
//! - Linux: a systemd user unit (`~/.config/systemd/user/lumisound-discord-rpc.service`)
//! - macOS: a LaunchAgent (`~/Library/LaunchAgents/com.lumisound.discord-rpc.plist`)
//! - Windows: a Scheduled Task (`LumisoundDiscordRPC`, at log-on, restarts on failure)
//!
//! The service runs this very binary (its current path), so put it somewhere
//! permanent before installing.

use std::path::{Path, PathBuf};
use std::process::Command;

#[cfg(all(unix, not(target_os = "macos")))]
const UNIT_NAME: &str = "lumisound-discord-rpc.service";
#[cfg(target_os = "macos")]
const LAUNCH_LABEL: &str = "com.lumisound.discord-rpc";
#[cfg(windows)]
const TASK_NAME: &str = "LumisoundDiscordRPC";

pub const HELP: &str = "lumisound-discord-rpc install [--config-only] [--force] [--bridge-url URL] [--token TOKEN] [--identifier EMAIL_OR_USERNAME]\n\
     lumisound-discord-rpc uninstall";

struct Opts {
    force: bool,
    config_only: bool,
    login_args: Vec<String>,
}

fn parse(args: &[String]) -> Result<Opts, String> {
    let mut o = Opts { force: false, config_only: false, login_args: vec![] };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--force" => o.force = true,
            "--config-only" => o.config_only = true,
            "--bridge-url" | "--token" | "--identifier" => {
                let v = it.next().ok_or_else(|| format!("{a} needs a value."))?;
                o.login_args.push(a.clone());
                o.login_args.push(v.clone());
            }
            other => return Err(format!("Unknown option: {other}\n{HELP}")),
        }
    }
    Ok(o)
}

fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

fn this_binary() -> Result<PathBuf, String> {
    let p = std::env::current_exe().map_err(|e| format!("Can't find this program's path: {e}"))?;
    Ok(p.canonicalize().unwrap_or(p))
}

fn run(cmd: &mut Command) -> Result<(), String> {
    let name = format!("{:?}", cmd.get_program());
    let st = cmd.status().map_err(|e| format!("{name} failed to start: {e}"))?;
    if st.success() { Ok(()) } else { Err(format!("{name} exited with {st}")) }
}

pub fn install(args: &[String]) -> Result<(), String> {
    let o = parse(args)?;
    let bin = this_binary()?;
    preflight(&o)?;
    crate::login::main(&o.login_args)?;
    if o.config_only {
        println!("\nConfig written; no service files touched (--config-only).");
        println!("If the service is already installed, restart it to pick up the new token.");
        return Ok(());
    }
    register(&bin)
}

pub fn uninstall(args: &[String]) -> Result<(), String> {
    if let Some(a) = args.first() {
        return Err(format!("Unknown option: {a}\n{HELP}"));
    }
    unregister()
}

// MARK: - Linux (systemd --user)

#[cfg(all(unix, not(target_os = "macos")))]
fn unit_dir() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| home().join(".config")).join("systemd").join("user")
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unit_text(bin: &Path) -> String {
    format!(
        "[Unit]\nDescription=Lumisound Discord Rich Presence bridge\nAfter=graphical-session.target\n\n\
         [Service]\nType=simple\nExecStart={}\nRestart=on-failure\nRestartSec=10\n\n\
         [Install]\nWantedBy=default.target\n",
        systemd_quote(&bin.to_string_lossy())
    )
}

/// ExecStart takes a command line; quote paths with spaces or specials.
#[cfg(all(unix, not(target_os = "macos")))]
fn systemd_quote(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        s.to_string()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%"))
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn preflight(o: &Opts) -> Result<(), String> {
    if unsafe { libc_geteuid() } == 0 {
        return Err("Don't run this as root: it installs a *user* service; as root it would configure root's account, not yours.".into());
    }
    if o.config_only {
        return Ok(());
    }
    let ok = Command::new("systemctl").args(["--user", "show-environment"]).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status();
    match ok {
        Err(_) => return Err("systemctl not found. This installs a systemd user service; use --config-only and start lumisound-discord-rpc however your system runs services.".into()),
        Ok(s) if !s.success() => {
            return Err("No systemd user session (systemctl --user can't reach its bus). On WSL enable systemd in /etc/wsl.conf; over SSH log in properly or use 'machinectl shell'. Or re-run with --config-only.".into())
        }
        Ok(_) => {}
    }
    let dest = unit_dir().join(UNIT_NAME);
    for managed in ["/etc/systemd/user", "/run/systemd/user", "/usr/lib/systemd/user"] {
        let m = Path::new(managed).join(UNIT_NAME);
        if m.exists() && !dest.is_symlink() {
            if o.force {
                eprintln!("Warning: {} exists; --force given, shadowing it with {}.", m.display(), dest.display());
            } else {
                return Err(format!(
                    "{UNIT_NAME} is already provided system-wide at {}.\n\
                     That usually means it's managed declaratively (NixOS, home-manager, Guix or a distro package);\n\
                     a unit in {} would override it and later rebuilds would silently have no effect.\n\
                     You probably want: lumisound-discord-rpc install --config-only\n\
                     To shadow the managed unit anyway, re-run with --force.",
                    m.display(),
                    unit_dir().display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
unsafe extern "C" {
    #[link_name = "geteuid"]
    fn libc_geteuid() -> u32;
}

#[cfg(all(unix, not(target_os = "macos")))]
fn register(bin: &Path) -> Result<(), String> {
    let dir = unit_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let dest = dir.join(UNIT_NAME);
    std::fs::write(&dest, unit_text(bin)).map_err(|e| format!("Couldn't write {}: {e}", dest.display()))?;
    println!("Installed {}", dest.display());
    run(Command::new("systemctl").args(["--user", "daemon-reload"]))?;
    run(Command::new("systemctl").args(["--user", "enable", "--now", UNIT_NAME]))?;
    // A reinstall over a running daemon must pick up the new token/binary.
    let _ = Command::new("systemctl").args(["--user", "restart", UNIT_NAME]).status();
    println!("\nService started. Check it with:\n  systemctl --user status {UNIT_NAME}\n  journalctl --user -u {UNIT_NAME} -f");
    Ok(())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn unregister() -> Result<(), String> {
    let _ = Command::new("systemctl").args(["--user", "disable", "--now", UNIT_NAME]).status();
    let dest = unit_dir().join(UNIT_NAME);
    if dest.exists() {
        std::fs::remove_file(&dest).map_err(|e| e.to_string())?;
        println!("Removed {}", dest.display());
    }
    let _ = Command::new("systemctl").args(["--user", "daemon-reload"]).status();
    println!("Uninstalled. Your sign-in (config.json) is kept; revoke it in Account → Active Sessions if you like.");
    Ok(())
}

// MARK: - macOS (LaunchAgent)

#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    home().join("Library").join("LaunchAgents").join(format!("{LAUNCH_LABEL}.plist"))
}

#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(target_os = "macos")]
fn plist_text(bin: &Path, log: &Path) -> String {
    let (b, l) = (xml_escape(&bin.to_string_lossy()), xml_escape(&log.to_string_lossy()));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n\
         \t<key>Label</key>\n\t<string>{LAUNCH_LABEL}</string>\n\
         \t<key>ProgramArguments</key>\n\t<array>\n\t\t<string>{b}</string>\n\t</array>\n\
         \t<key>RunAtLoad</key>\n\t<true/>\n\
         \t<key>KeepAlive</key>\n\t<true/>\n\
         \t<key>StandardOutPath</key>\n\t<string>{l}</string>\n\
         \t<key>StandardErrorPath</key>\n\t<string>{l}</string>\n\
         </dict>\n</plist>\n"
    )
}

#[cfg(target_os = "macos")]
fn preflight(_o: &Opts) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "macos")]
fn register(bin: &Path) -> Result<(), String> {
    let log_dir = crate::default_config_path().parent().map(Path::to_path_buf).unwrap_or_else(|| home().join(".config/lumisound-discord-rpc"));
    std::fs::create_dir_all(&log_dir).map_err(|e| e.to_string())?;
    let plist = plist_path();
    if let Some(d) = plist.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let log = log_dir.join("daemon.log");
    std::fs::write(&plist, plist_text(bin, &log)).map_err(|e| format!("Couldn't write {}: {e}", plist.display()))?;
    println!("Installed {}", plist.display());
    let _ = Command::new("launchctl").arg("unload").arg(&plist).stderr(std::process::Stdio::null()).status();
    run(Command::new("launchctl").arg("load").arg(&plist))?;
    println!("\nStarted. Check it with:\n  launchctl list | grep lumisound\n  tail -f {}", log.display());
    Ok(())
}

#[cfg(target_os = "macos")]
fn unregister() -> Result<(), String> {
    let plist = plist_path();
    if plist.exists() {
        let _ = Command::new("launchctl").arg("unload").arg(&plist).status();
        std::fs::remove_file(&plist).map_err(|e| e.to_string())?;
        println!("Removed {}", plist.display());
    }
    println!("Uninstalled. Your sign-in (config.json) is kept; revoke it in Account → Active Sessions if you like.");
    Ok(())
}

// MARK: - Windows (Scheduled Task)

#[cfg(windows)]
fn preflight(_o: &Opts) -> Result<(), String> {
    Ok(())
}

/// Task Scheduler XML: at log-on, hidden, no time limit, restart every
/// minute on failure (what the PowerShell script registered).
#[cfg(windows)]
fn task_xml(bin: &Path) -> String {
    let b = bin.to_string_lossy().replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-16\"?>\r\n\
         <Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\">\r\n\
         <Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers>\r\n\
         <Principals><Principal id=\"Author\"><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals>\r\n\
         <Settings><Hidden>true</Hidden><ExecutionTimeLimit>PT0S</ExecutionTimeLimit>\
         <RestartOnFailure><Interval>PT1M</Interval><Count>999</Count></RestartOnFailure>\
         <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>\
         <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy></Settings>\r\n\
         <Actions Context=\"Author\"><Exec><Command>{b}</Command></Exec></Actions>\r\n\
         </Task>\r\n"
    )
}

#[cfg(windows)]
fn register(bin: &Path) -> Result<(), String> {
    // schtasks wants the XML as UTF-16 LE with a BOM.
    let xml = task_xml(bin);
    let mut bytes = vec![0xFF, 0xFE];
    for u in xml.encode_utf16() {
        bytes.extend_from_slice(&u.to_le_bytes());
    }
    let tmp = std::env::temp_dir().join("lumisound-discord-rpc-task.xml");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    let r = run(Command::new("schtasks").args(["/Create", "/F", "/TN", TASK_NAME, "/XML"]).arg(&tmp));
    let _ = std::fs::remove_file(&tmp);
    r?;
    println!("Registered scheduled task '{TASK_NAME}' (runs at log-on). Starting it now...");
    run(Command::new("schtasks").args(["/Run", "/TN", TASK_NAME]))?;
    println!("\nCheck it with:\n  schtasks /Query /TN {TASK_NAME}\nStop it with:\n  lumisound-discord-rpc uninstall");
    Ok(())
}

#[cfg(windows)]
fn unregister() -> Result<(), String> {
    let _ = Command::new("schtasks").args(["/End", "/TN", TASK_NAME]).status();
    let _ = Command::new("schtasks").args(["/Delete", "/F", "/TN", TASK_NAME]).status();
    println!("Uninstalled. Your sign-in (config.json) is kept; revoke it in Account → Active Sessions if you like.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_options() {
        let a: Vec<String> = ["--config-only", "--bridge-url", "https://x", "--identifier", "me"].iter().map(|s| s.to_string()).collect();
        let o = parse(&a).unwrap();
        assert!(o.config_only && !o.force);
        assert_eq!(o.login_args, vec!["--bridge-url", "https://x", "--identifier", "me"]);
        assert!(parse(&["--token".to_string()]).is_err());
        assert!(parse(&["--nope".to_string()]).is_err());
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn unit_quotes_odd_paths() {
        assert!(unit_text(Path::new("/home/a/.local/bin/lumisound-discord-rpc")).contains("ExecStart=/home/a/.local/bin/lumisound-discord-rpc\n"));
        assert!(unit_text(Path::new("/home/a b/rpc")).contains("ExecStart=\"/home/a b/rpc\"\n"));
    }
}
