use clap::{Parser, Subcommand};
use evdev::KeyCode;
use serde::Deserialize;
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::str::FromStr;

#[derive(Parser, Debug)]
#[command(
    about = "Userspace daemon for the Panasonic Let's Note circular trackpad",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,

    /// Input device path [default: auto-detect]
    #[arg(short, long)]
    pub device: Option<PathBuf>,

    /// Pointer sensitivity (multiplier on raw ABS deltas)
    #[arg(short, long)]
    pub pointer: Option<f64>,

    /// Scroll sensitivity (REL_WHEEL ticks per radian)
    #[arg(short, long)]
    pub scroll: Option<f64>,

    /// Ring threshold as fraction of max radius (0.0-1.0)
    #[arg(short, long)]
    pub ring: Option<f64>,

    /// Invert scroll direction
    #[arg(short, long, conflicts_with = "no_invert_scroll")]
    pub invert_scroll: bool,

    /// Do not invert scroll direction
    #[arg(long, conflicts_with = "invert_scroll")]
    pub no_invert_scroll: bool,

    /// Enable tap-to-click
    #[arg(long, conflicts_with = "no_tap")]
    pub tap: bool,

    /// Disable tap-to-click and restore physical left/right buttons
    #[arg(long, conflicts_with = "tap")]
    pub no_tap: bool,

    /// Tap timeout in milliseconds
    #[arg(long)]
    pub tap_timeout: Option<u64>,

    /// Tap movement threshold in raw coordinate units
    #[arg(long)]
    pub tap_move_threshold: Option<i32>,
}

#[derive(Subcommand, Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliCommand {
    /// Validate the config and restart the systemd user service
    Restart,
}

impl Cli {
    pub fn has_daemon_overrides(&self) -> bool {
        self.device.is_some()
            || self.pointer.is_some()
            || self.scroll.is_some()
            || self.ring.is_some()
            || self.invert_scroll
            || self.no_invert_scroll
            || self.tap
            || self.no_tap
            || self.tap_timeout.is_some()
            || self.tap_move_threshold.is_some()
    }
}

#[derive(Debug)]
pub struct ConfigError(String);

impl ConfigError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut(pub Vec<KeyCode>);

impl Shortcut {
    fn parse(field: &str, names: Vec<String>) -> Result<Self, ConfigError> {
        if names.is_empty() {
            return Err(ConfigError::new(format!("{field} must not be empty")));
        }

        let mut keys = Vec::with_capacity(names.len());
        let mut seen = HashSet::new();
        for name in names {
            if !name.starts_with("KEY_") {
                return Err(ConfigError::new(format!(
                    "{field}: '{name}' is not a Linux KEY_* name"
                )));
            }
            let key = KeyCode::from_str(&name).map_err(|_| {
                ConfigError::new(format!("{field}: unknown Linux key name '{name}'"))
            })?;
            if !seen.insert(key.code()) {
                return Err(ConfigError::new(format!("{field}: duplicate key '{name}'")));
            }
            keys.push(key);
        }
        Ok(Self(keys))
    }
}

#[derive(Debug, Clone)]
pub struct ButtonGestures {
    pub step_degrees: f64,
    pub left_clockwise: Shortcut,
    pub left_counterclockwise: Shortcut,
    pub right_clockwise: Shortcut,
    pub right_counterclockwise: Shortcut,
}

#[derive(Debug, Clone)]
pub struct TwoFingerSwipe {
    pub enabled: bool,
    pub distance: i32,
    pub up: Shortcut,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub path: PathBuf,
    pub file_found: bool,
    pub device: Option<PathBuf>,
    pub pointer: f64,
    pub scroll: f64,
    pub ring: f64,
    pub invert_scroll: bool,
    pub tap: bool,
    pub tap_timeout_ms: u64,
    pub tap_move_threshold: i32,
    pub button_gestures: ButtonGestures,
    pub two_finger_swipe: TwoFingerSwipe,
}

impl Config {
    pub fn keyboard_keys(&self) -> HashSet<KeyCode> {
        let mut keys = HashSet::new();
        for shortcut in [
            &self.button_gestures.left_clockwise,
            &self.button_gestures.left_counterclockwise,
            &self.button_gestures.right_clockwise,
            &self.button_gestures.right_counterclockwise,
            &self.two_finger_swipe.up,
        ] {
            keys.extend(shortcut.0.iter().copied());
        }
        keys
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileConfig {
    device: Option<String>,
    pointer: Option<f64>,
    scroll: Option<f64>,
    ring: Option<f64>,
    invert_scroll: Option<bool>,
    tap: Option<bool>,
    tap_timeout_ms: Option<u64>,
    tap_move_threshold: Option<i32>,
    button_gestures: Option<FileButtonGestures>,
    two_finger_swipe: Option<FileTwoFingerSwipe>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileButtonGestures {
    step_degrees: Option<f64>,
    left_clockwise: Option<Vec<String>>,
    left_counterclockwise: Option<Vec<String>>,
    right_clockwise: Option<Vec<String>>,
    right_counterclockwise: Option<Vec<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FileTwoFingerSwipe {
    enabled: Option<bool>,
    distance: Option<i32>,
    up: Option<Vec<String>>,
}

pub fn config_path() -> Result<PathBuf, ConfigError> {
    config_path_from(env::var_os("XDG_CONFIG_HOME"), env::var_os("HOME"))
}

fn config_path_from(
    xdg_config_home: Option<OsString>,
    home: Option<OsString>,
) -> Result<PathBuf, ConfigError> {
    if let Some(value) = xdg_config_home.filter(|v| !v.is_empty()) {
        let base = PathBuf::from(value);
        if !base.is_absolute() {
            return Err(ConfigError::new("XDG_CONFIG_HOME must be an absolute path"));
        }
        return Ok(base.join("circulartrackpad/config.toml"));
    }

    let home = home
        .filter(|v| !v.is_empty())
        .ok_or_else(|| ConfigError::new("HOME is not set; cannot locate user configuration"))?;
    Ok(PathBuf::from(home).join(".config/circulartrackpad/config.toml"))
}

pub fn load(cli: &Cli) -> Result<Config, ConfigError> {
    let path = config_path()?;
    load_from_path(cli, &path)
}

pub fn load_from_path(cli: &Cli, path: &Path) -> Result<Config, ConfigError> {
    let (file, file_found) = match fs::read_to_string(path) {
        Ok(contents) => {
            let parsed = toml::from_str::<FileConfig>(&contents).map_err(|error| {
                ConfigError::new(format!("invalid config '{}': {error}", path.display()))
            })?;
            (parsed, true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            (FileConfig::default(), false)
        }
        Err(error) => {
            return Err(ConfigError::new(format!(
                "cannot read config '{}': {error}",
                path.display()
            )));
        }
    };

    let button = file.button_gestures.unwrap_or_default();
    let swipe = file.two_finger_swipe.unwrap_or_default();

    let pointer = cli.pointer.or(file.pointer).unwrap_or(1.5);
    let scroll = cli.scroll.or(file.scroll).unwrap_or(5.0);
    let ring = cli.ring.or(file.ring).unwrap_or(0.65);
    let invert_scroll = if cli.invert_scroll {
        true
    } else if cli.no_invert_scroll {
        false
    } else {
        file.invert_scroll.unwrap_or(false)
    };
    let tap = if cli.tap {
        true
    } else if cli.no_tap {
        false
    } else {
        file.tap.unwrap_or(true)
    };
    let tap_timeout_ms = cli.tap_timeout.or(file.tap_timeout_ms).unwrap_or(180);
    let tap_move_threshold = cli
        .tap_move_threshold
        .or(file.tap_move_threshold)
        .unwrap_or(20);

    validate_finite_positive("pointer", pointer, false)?;
    validate_finite_positive("scroll", scroll, true)?;
    if !ring.is_finite() || !(0.0..=1.0).contains(&ring) {
        return Err(ConfigError::new("ring must be finite and within 0.0..=1.0"));
    }
    if tap_timeout_ms == 0 {
        return Err(ConfigError::new("tap_timeout_ms must be greater than zero"));
    }
    if tap_move_threshold < 0 {
        return Err(ConfigError::new(
            "tap_move_threshold must be zero or greater",
        ));
    }

    let step_degrees = button.step_degrees.unwrap_or(30.0);
    if !step_degrees.is_finite() || !(0.0 < step_degrees && step_degrees <= 360.0) {
        return Err(ConfigError::new(
            "button_gestures.step_degrees must be within 0.0 (exclusive)..=360.0",
        ));
    }

    let swipe_distance = swipe.distance.unwrap_or(80);
    if swipe_distance <= tap_move_threshold {
        return Err(ConfigError::new(
            "two_finger_swipe.distance must be greater than tap_move_threshold",
        ));
    }

    let device = cli
        .device
        .clone()
        .or_else(|| file.device.map(PathBuf::from));
    if device
        .as_ref()
        .is_some_and(|path| path.as_os_str().is_empty())
    {
        return Err(ConfigError::new("device must not be empty"));
    }

    Ok(Config {
        path: path.to_path_buf(),
        file_found,
        device,
        pointer,
        scroll,
        ring,
        invert_scroll,
        tap,
        tap_timeout_ms,
        tap_move_threshold,
        button_gestures: ButtonGestures {
            step_degrees,
            left_clockwise: Shortcut::parse(
                "button_gestures.left_clockwise",
                button
                    .left_clockwise
                    .unwrap_or_else(|| key_names(&["KEY_LEFTALT", "KEY_ESC"])),
            )?,
            left_counterclockwise: Shortcut::parse(
                "button_gestures.left_counterclockwise",
                button
                    .left_counterclockwise
                    .unwrap_or_else(|| key_names(&["KEY_LEFTSHIFT", "KEY_LEFTALT", "KEY_ESC"])),
            )?,
            right_clockwise: Shortcut::parse(
                "button_gestures.right_clockwise",
                button
                    .right_clockwise
                    .unwrap_or_else(|| key_names(&["KEY_LEFTMETA", "KEY_PAGEDOWN"])),
            )?,
            right_counterclockwise: Shortcut::parse(
                "button_gestures.right_counterclockwise",
                button
                    .right_counterclockwise
                    .unwrap_or_else(|| key_names(&["KEY_LEFTMETA", "KEY_PAGEUP"])),
            )?,
        },
        two_finger_swipe: TwoFingerSwipe {
            enabled: swipe.enabled.unwrap_or(true),
            distance: swipe_distance,
            up: Shortcut::parse(
                "two_finger_swipe.up",
                swipe.up.unwrap_or_else(|| key_names(&["KEY_LEFTMETA"])),
            )?,
        },
    })
}

fn key_names(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

fn validate_finite_positive(field: &str, value: f64, allow_zero: bool) -> Result<(), ConfigError> {
    let valid_sign = if allow_zero {
        value >= 0.0
    } else {
        value > 0.0
    };
    if value.is_finite() && valid_sign {
        Ok(())
    } else {
        let qualifier = if allow_zero {
            "zero or greater"
        } else {
            "greater than zero"
        };
        Err(ConfigError::new(format!(
            "{field} must be finite and {qualifier}"
        )))
    }
}

pub fn restart_service() -> Result<(), ConfigError> {
    restart_service_with(|program, args| {
        ProcessCommand::new(program)
            .args(args)
            .status()
            .map_err(|error| error.to_string())
            .and_then(|status| {
                if status.success() {
                    Ok(())
                } else {
                    Err(format!("systemctl exited with {status}"))
                }
            })
    })
}

fn restart_service_with<F>(mut runner: F) -> Result<(), ConfigError>
where
    F: FnMut(&str, &[&str]) -> Result<(), String>,
{
    runner(
        "systemctl",
        &["--user", "restart", "circulartrackpad.service"],
    )
    .map_err(|error| ConfigError::new(format!("failed to restart service: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn cli() -> Cli {
        Cli::parse_from(["circulartrackpad"])
    }

    fn temp_config(contents: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = env::temp_dir().join(format!("circulartrackpad-{unique}.toml"));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn missing_file_uses_defaults() {
        let path = env::temp_dir().join("circulartrackpad-does-not-exist.toml");
        let config = load_from_path(&cli(), &path).unwrap();
        assert!(!config.file_found);
        assert_eq!(config.pointer, 1.5);
        assert_eq!(config.two_finger_swipe.distance, 80);
    }

    #[test]
    fn config_path_prefers_xdg_and_falls_back_to_home() {
        assert_eq!(
            config_path_from(
                Some(OsString::from("/xdg")),
                Some(OsString::from("/home/me"))
            )
            .unwrap(),
            PathBuf::from("/xdg/circulartrackpad/config.toml")
        );
        assert_eq!(
            config_path_from(None, Some(OsString::from("/home/me"))).unwrap(),
            PathBuf::from("/home/me/.config/circulartrackpad/config.toml")
        );
        assert!(config_path_from(Some(OsString::from("relative")), None).is_err());
    }

    #[test]
    fn partial_file_and_cli_precedence() {
        let path = temp_config("pointer = 2.0\ninvert_scroll = true\ntap = false\n");
        let parsed = Cli::parse_from([
            "circulartrackpad",
            "--pointer",
            "3.0",
            "--no-invert-scroll",
            "--tap",
        ]);
        let config = load_from_path(&parsed, &path).unwrap();
        assert_eq!(config.pointer, 3.0);
        assert!(!config.invert_scroll);
        assert!(config.tap);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_unknown_fields_and_keys() {
        let unknown_field = temp_config("typo = true\n");
        assert!(load_from_path(&cli(), &unknown_field)
            .unwrap_err()
            .to_string()
            .contains("unknown field"));
        fs::remove_file(unknown_field).unwrap();

        let unknown_key =
            temp_config("[button_gestures]\nleft_clockwise = [\"KEY_NOT_A_REAL_KEY\"]\n");
        assert!(load_from_path(&cli(), &unknown_key)
            .unwrap_err()
            .to_string()
            .contains("unknown Linux key"));
        fs::remove_file(unknown_key).unwrap();
    }

    #[test]
    fn rejects_malformed_empty_and_duplicate_shortcuts() {
        let malformed = temp_config("[button_gestures\n");
        assert!(load_from_path(&cli(), &malformed).is_err());
        fs::remove_file(malformed).unwrap();

        let empty = temp_config("[two_finger_swipe]\nup = []\n");
        assert!(load_from_path(&cli(), &empty)
            .unwrap_err()
            .to_string()
            .contains("must not be empty"));
        fs::remove_file(empty).unwrap();

        let duplicate =
            temp_config("[two_finger_swipe]\nup = [\"KEY_LEFTMETA\", \"KEY_LEFTMETA\"]\n");
        assert!(load_from_path(&cli(), &duplicate)
            .unwrap_err()
            .to_string()
            .contains("duplicate key"));
        fs::remove_file(duplicate).unwrap();
    }

    #[test]
    fn rejects_invalid_ranges_and_threshold_order() {
        let ring = temp_config("ring = 1.1\n");
        assert!(load_from_path(&cli(), &ring).is_err());
        fs::remove_file(ring).unwrap();

        let swipe = temp_config("tap_move_threshold = 20\n[two_finger_swipe]\ndistance = 20\n");
        assert!(load_from_path(&cli(), &swipe).is_err());
        fs::remove_file(swipe).unwrap();
    }

    #[test]
    fn keyboard_capabilities_include_every_shortcut_key() {
        let config = load_from_path(&cli(), Path::new("/missing")).unwrap();
        let keys = config.keyboard_keys();
        assert!(keys.contains(&KeyCode::KEY_LEFTALT));
        assert!(keys.contains(&KeyCode::KEY_LEFTMETA));
        assert!(keys.contains(&KeyCode::KEY_PAGEDOWN));
    }

    #[test]
    fn restart_invokes_exact_systemctl_command() {
        let invocation = RefCell::new(None);
        restart_service_with(|program, args| {
            *invocation.borrow_mut() = Some((program.to_string(), args.join(" ")));
            Ok(())
        })
        .unwrap();
        assert_eq!(
            invocation.into_inner(),
            Some((
                "systemctl".to_string(),
                "--user restart circulartrackpad.service".to_string()
            ))
        );
    }
}
