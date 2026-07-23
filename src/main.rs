mod config;
mod gestures;

use clap::Parser;
use config::{Cli, CliCommand, Config};
use evdev::uinput::VirtualDevice;
use evdev::{
    AttributeSet, Device, EventType, InputEvent, KeyCode, RelativeAxisCode, SynchronizationCode,
};
use gestures::{DriverState, Event, Output, PhysicalButton, PointerEvent};
use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

const TRACKPAD_NAME: &str = "Synaptics TM3562-003";
const ABS_MT_SLOT: u16 = 0x2f;
const ABS_MT_TRACKING_ID: u16 = 0x39;
const ABS_MT_POSITION_X: u16 = 0x35;
const ABS_MT_POSITION_Y: u16 = 0x36;

fn main() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    if cli.command == Some(CliCommand::Restart) {
        if cli.has_daemon_overrides() {
            return Err("daemon options cannot be combined with 'restart'".into());
        }
        let config = config::load(&cli)?;
        println!(
            "circulartrackpad: configuration valid ({})",
            config.path.display()
        );
        config::restart_service()?;
        println!("circulartrackpad: restarted circulartrackpad.service");
        return Ok(());
    }

    let config = config::load(&cli)?;
    run(config)
}

fn run(config: Config) -> Result<(), Box<dyn Error>> {
    if config.file_found {
        println!(
            "circulartrackpad: using configuration {}",
            config.path.display()
        );
    } else {
        println!(
            "circulartrackpad: configuration {} not found; using defaults and CLI options",
            config.path.display()
        );
    }

    let device_path = match &config.device {
        Some(path) => path.clone(),
        None => find_trackpad().ok_or_else(|| {
            let mut message = format!(
                "Could not auto-detect '{}' trackpad.\n\nAvailable input devices:\n",
                TRACKPAD_NAME
            );
            for (path, name) in list_input_devices() {
                message.push_str(&format!(
                    "  {}  {}\n",
                    path.display(),
                    name.as_deref().unwrap_or("(no name)")
                ));
            }
            message.push_str("\nPass the correct node explicitly with -d /dev/input/eventN.");
            message
        })?,
    };

    println!("circulartrackpad: opening {}", device_path.display());
    let mut device = Device::open(&device_path)?;
    println!(
        "circulartrackpad: grabbed '{}' (pointer={}, scroll={}, ring={}, tap={})",
        device.name().unwrap_or("unknown"),
        config.pointer,
        config.scroll,
        config.ring,
        config.tap
    );
    device.grab()?;

    let mut pointer = build_virtual_pointer()?;
    let mut keyboard = build_virtual_keyboard(&config)?;
    let mut state = DriverState::new(&config);
    println!("circulartrackpad: virtual pointer and shortcut keyboard created");

    loop {
        for input in device.fetch_events()? {
            let now = Instant::now();
            let event = match input.event_type() {
                EventType::ABSOLUTE => match input.code() {
                    ABS_MT_SLOT if input.value() >= 0 => {
                        Some(Event::SelectSlot(input.value() as usize))
                    }
                    ABS_MT_TRACKING_ID => Some(Event::TrackingId(input.value())),
                    ABS_MT_POSITION_X => Some(Event::PositionX(input.value())),
                    ABS_MT_POSITION_Y => Some(Event::PositionY(input.value())),
                    _ => None,
                },
                EventType::KEY => match input.code() {
                    code if code == KeyCode::BTN_LEFT.code() => {
                        Some(Event::Button(PhysicalButton::Left, input.value()))
                    }
                    code if code == KeyCode::BTN_RIGHT.code() => {
                        Some(Event::Button(PhysicalButton::Right, input.value()))
                    }
                    code if code == KeyCode::BTN_MIDDLE.code() => {
                        Some(Event::Button(PhysicalButton::Middle, input.value()))
                    }
                    _ => None,
                },
                EventType::SYNCHRONIZATION if input.code() == SynchronizationCode::SYN_REPORT.0 => {
                    Some(Event::Frame)
                }
                _ => None,
            };

            if let Some(event) = event {
                emit_outputs(&mut pointer, &mut keyboard, state.process(event, now))?;
            }
        }
    }
}

fn build_virtual_pointer() -> Result<VirtualDevice, Box<dyn Error>> {
    let mut keys = AttributeSet::<KeyCode>::new();
    keys.insert(KeyCode::BTN_LEFT);
    keys.insert(KeyCode::BTN_RIGHT);
    keys.insert(KeyCode::BTN_MIDDLE);

    let mut relative_axes = AttributeSet::<RelativeAxisCode>::new();
    relative_axes.insert(RelativeAxisCode::REL_X);
    relative_axes.insert(RelativeAxisCode::REL_Y);
    relative_axes.insert(RelativeAxisCode::REL_WHEEL);
    relative_axes.insert(RelativeAxisCode::REL_HWHEEL);
    relative_axes.insert(RelativeAxisCode::REL_WHEEL_HI_RES);
    relative_axes.insert(RelativeAxisCode::REL_HWHEEL_HI_RES);

    Ok(VirtualDevice::builder()?
        .name("circulartrackpad")
        .with_keys(&keys)?
        .with_relative_axes(&relative_axes)?
        .build()?)
}

fn build_virtual_keyboard(config: &Config) -> Result<VirtualDevice, Box<dyn Error>> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for key in config.keyboard_keys() {
        keys.insert(key);
    }
    Ok(VirtualDevice::builder()?
        .name("circulartrackpad shortcuts")
        .with_keys(&keys)?
        .build()?)
}

fn emit_outputs(
    pointer: &mut VirtualDevice,
    keyboard: &mut VirtualDevice,
    outputs: Vec<Output>,
) -> Result<(), Box<dyn Error>> {
    for output in outputs {
        match output {
            Output::PointerFrame(events) => {
                let events: Vec<_> = events.into_iter().map(pointer_input_event).collect();
                pointer.emit(&events)?;
            }
            Output::Shortcut(shortcut) => {
                let pressed: Vec<_> = shortcut
                    .0
                    .iter()
                    .map(|key| InputEvent::new(EventType::KEY.0, key.code(), 1))
                    .collect();
                keyboard.emit(&pressed)?;

                let released: Vec<_> = shortcut
                    .0
                    .iter()
                    .rev()
                    .map(|key| InputEvent::new(EventType::KEY.0, key.code(), 0))
                    .collect();
                keyboard.emit(&released)?;
            }
        }
    }
    Ok(())
}

fn pointer_input_event(event: PointerEvent) -> InputEvent {
    match event {
        PointerEvent::RelX(value) => {
            InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_X.0, value)
        }
        PointerEvent::RelY(value) => {
            InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_Y.0, value)
        }
        PointerEvent::WheelHiRes(value) => InputEvent::new(
            EventType::RELATIVE.0,
            RelativeAxisCode::REL_WHEEL_HI_RES.0,
            value,
        ),
        PointerEvent::Wheel(value) => {
            InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_WHEEL.0, value)
        }
        PointerEvent::Button(button, value) => {
            let key = match button {
                PhysicalButton::Left => KeyCode::BTN_LEFT,
                PhysicalButton::Right => KeyCode::BTN_RIGHT,
                PhysicalButton::Middle => KeyCode::BTN_MIDDLE,
            };
            InputEvent::new(EventType::KEY.0, key.code(), value)
        }
    }
}

fn find_trackpad() -> Option<PathBuf> {
    evdev::enumerate()
        .find_map(|(path, device)| (device.name() == Some(TRACKPAD_NAME)).then_some(path))
}

fn list_input_devices() -> Vec<(PathBuf, Option<String>)> {
    evdev::enumerate()
        .map(|(path, device)| (path, device.name().map(str::to_owned)))
        .collect()
}
