mod config;
mod gestures;

use clap::Parser;
use config::{Cli, CliCommand, Config};
use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, Device, EventType, InputEvent, KeyCode, PropType,
    RelativeAxisCode, SynchronizationCode, UinputAbsSetup,
};
use gestures::{DriverState, Event, NativeContact, Output, PhysicalButton, PointerEvent};
use std::error::Error;
use std::path::PathBuf;
use std::time::Instant;

const TRACKPAD_NAME: &str = "Synaptics TM3562-003";
const GESTURE_DEVICE_NAME: &str = "circulartrackpad gestures";
const PAD_MAX: i32 = 528;
const PAD_RESOLUTION: i32 = 12;
const NATIVE_SLOT_COUNT: usize = 4;
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
        print_config_warnings(&config);
        println!(
            "circulartrackpad: configuration valid ({})",
            config.path.display()
        );
        config::restart_service()?;
        println!("circulartrackpad: restarted circulartrackpad.service");
        return Ok(());
    }

    let config = config::load(&cli)?;
    print_config_warnings(&config);
    run(config)
}

fn print_config_warnings(config: &Config) {
    for warning in &config.warnings {
        eprintln!("circulartrackpad: warning: {warning}");
    }
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
    let mut native_gestures = if config.native_gestures.enabled {
        Some(build_virtual_gesture_touchpad()?)
    } else {
        None
    };
    let mut native_state = NativeOutputState::default();
    let mut state = DriverState::new(&config);
    if native_gestures.is_some() {
        println!(
            "circulartrackpad: virtual pointer, shortcut keyboard, and native gesture touchpad created"
        );
    } else {
        println!("circulartrackpad: virtual pointer and shortcut keyboard created");
    }

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
                emit_outputs(
                    &mut pointer,
                    &mut keyboard,
                    native_gestures.as_mut(),
                    &mut native_state,
                    state.process(event, now),
                )?;
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

fn build_virtual_gesture_touchpad() -> Result<VirtualDevice, Box<dyn Error>> {
    let mut keys = AttributeSet::<KeyCode>::new();
    for key in [
        KeyCode::BTN_TOUCH,
        KeyCode::BTN_TOOL_FINGER,
        KeyCode::BTN_TOOL_DOUBLETAP,
        KeyCode::BTN_TOOL_TRIPLETAP,
        KeyCode::BTN_TOOL_QUADTAP,
    ] {
        keys.insert(key);
    }

    let mut properties = AttributeSet::<PropType>::new();
    properties.insert(PropType::POINTER);

    let coordinate = AbsInfo::new(0, 0, PAD_MAX, 0, 0, PAD_RESOLUTION);
    let slot = AbsInfo::new(0, 0, NATIVE_SLOT_COUNT as i32 - 1, 0, 0, 0);
    let tracking_id = AbsInfo::new(0, 0, 65_535, 0, 0, 0);
    let axes = [
        UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, coordinate),
        UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, coordinate),
        UinputAbsSetup::new(AbsoluteAxisCode::ABS_MT_SLOT, slot),
        UinputAbsSetup::new(AbsoluteAxisCode::ABS_MT_TRACKING_ID, tracking_id),
        UinputAbsSetup::new(AbsoluteAxisCode::ABS_MT_POSITION_X, coordinate),
        UinputAbsSetup::new(AbsoluteAxisCode::ABS_MT_POSITION_Y, coordinate),
    ];

    let mut builder = VirtualDevice::builder()?
        .name(GESTURE_DEVICE_NAME)
        .with_keys(&keys)?
        .with_properties(&properties)?;
    for axis in &axes {
        builder = builder.with_absolute_axis(axis)?;
    }
    Ok(builder.build()?)
}

#[derive(Default)]
struct NativeOutputState {
    tracking_ids: [Option<i32>; NATIVE_SLOT_COUNT],
}

fn emit_outputs(
    pointer: &mut VirtualDevice,
    keyboard: &mut VirtualDevice,
    native_gestures: Option<&mut VirtualDevice>,
    native_state: &mut NativeOutputState,
    outputs: Vec<Output>,
) -> Result<(), Box<dyn Error>> {
    let mut native_gestures = native_gestures;
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
            Output::NativeFrame(contacts) => {
                let device = native_gestures
                    .as_deref_mut()
                    .ok_or("native gesture output requested while native gestures are disabled")?;
                emit_native_frame(device, native_state, &contacts)?;
            }
        }
    }
    Ok(())
}

fn emit_native_frame(
    device: &mut VirtualDevice,
    state: &mut NativeOutputState,
    contacts: &[NativeContact],
) -> Result<(), Box<dyn Error>> {
    if contacts.len() > NATIVE_SLOT_COUNT {
        return Err("native gesture frame exceeds four contacts".into());
    }

    let mut by_slot = [None; NATIVE_SLOT_COUNT];
    for contact in contacts {
        if contact.slot >= NATIVE_SLOT_COUNT {
            return Err(format!("invalid native gesture slot {}", contact.slot).into());
        }
        if by_slot[contact.slot].replace(*contact).is_some() {
            return Err(format!("duplicate native gesture slot {}", contact.slot).into());
        }
    }

    let mut events = Vec::new();
    for (slot, contact) in by_slot.iter().enumerate() {
        events.push(absolute_event(
            AbsoluteAxisCode::ABS_MT_SLOT,
            slot as i32,
        ));
        let next_tracking_id = contact.as_ref().map(|contact| contact.tracking_id);
        if state.tracking_ids[slot] != next_tracking_id {
            if state.tracking_ids[slot].is_some() {
                events.push(absolute_event(
                    AbsoluteAxisCode::ABS_MT_TRACKING_ID,
                    -1,
                ));
            }
            if let Some(tracking_id) = next_tracking_id {
                events.push(absolute_event(
                    AbsoluteAxisCode::ABS_MT_TRACKING_ID,
                    tracking_id,
                ));
            }
            state.tracking_ids[slot] = next_tracking_id;
        }
        if let Some(contact) = contact.as_ref() {
            events.push(absolute_event(
                AbsoluteAxisCode::ABS_MT_POSITION_X,
                contact.x,
            ));
            events.push(absolute_event(
                AbsoluteAxisCode::ABS_MT_POSITION_Y,
                contact.y,
            ));
        }
    }

    if let Some(primary) = contacts.first() {
        events.push(absolute_event(AbsoluteAxisCode::ABS_X, primary.x));
        events.push(absolute_event(AbsoluteAxisCode::ABS_Y, primary.y));
    }

    let count = contacts.len();
    for (key, value) in [
        (KeyCode::BTN_TOUCH, i32::from(count > 0)),
        (KeyCode::BTN_TOOL_FINGER, i32::from(count == 1)),
        (KeyCode::BTN_TOOL_DOUBLETAP, i32::from(count == 2)),
        (KeyCode::BTN_TOOL_TRIPLETAP, i32::from(count == 3)),
        (KeyCode::BTN_TOOL_QUADTAP, i32::from(count == 4)),
    ] {
        events.push(InputEvent::new(EventType::KEY.0, key.code(), value));
    }
    device.emit(&events)?;
    Ok(())
}

fn absolute_event(axis: AbsoluteAxisCode, value: i32) -> InputEvent {
    InputEvent::new(EventType::ABSOLUTE.0, axis.0, value)
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
