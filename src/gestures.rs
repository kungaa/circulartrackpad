use crate::config::{Config, Shortcut};
use std::f64::consts::PI;
use std::time::{Duration, Instant};

const PAD_MAX: i32 = 528;
const CENTER_X: f64 = PAD_MAX as f64 / 2.0;
const CENTER_Y: f64 = PAD_MAX as f64 / 2.0;
const MAX_RADIUS: f64 = PAD_MAX as f64 / 2.0;
const SLOT_COUNT: usize = 5;
const NATIVE_SLOT_COUNT: usize = 4;
const SYNTHETIC_CONTACT_OFFSET: i32 = 48;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointerEvent {
    RelX(i32),
    RelY(i32),
    WheelHiRes(i32),
    Wheel(i32),
    Button(PhysicalButton, i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NativeContact {
    pub slot: usize,
    pub tracking_id: i32,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    PointerFrame(Vec<PointerEvent>),
    Shortcut(Shortcut),
    NativeFrame(Vec<NativeContact>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    SelectSlot(usize),
    TrackingId(i32),
    PositionX(i32),
    PositionY(i32),
    Button(PhysicalButton, i32),
    Frame,
}

#[derive(Debug, Clone, Copy)]
struct SlotState {
    tracking_id: i32,
    x: i32,
    y: i32,
}

impl Default for SlotState {
    fn default() -> Self {
        Self {
            tracking_id: -1,
            x: 0,
            y: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct TapState {
    start_time: Option<Instant>,
    start_pos: Option<(i32, i32)>,
    moved: bool,
    canceled: bool,
}

impl TapState {
    fn valid(&self, now: Instant, timeout: Duration) -> bool {
        self.start_time
            .is_some_and(|start| now.duration_since(start) <= timeout)
            && !self.moved
            && !self.canceled
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Inner,
    Ring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockedDirection {
    Left,
    Right,
    Vertical,
}

#[derive(Debug, Clone)]
enum SwipeState {
    Idle,
    Two {
        start: [(i32, i32); 2],
        direction: Option<LockedDirection>,
        horizontal_triggered: bool,
    },
    NativeTwo {
        synthetic_offset: i32,
    },
    NativeMulti {
        mapping: [Option<i32>; NATIVE_SLOT_COUNT],
    },
    Blocked,
}

impl SwipeState {
    fn native_active(&self) -> bool {
        matches!(self, Self::NativeTwo { .. } | Self::NativeMulti { .. })
    }
}

#[derive(Debug, Clone)]
struct Settings {
    pointer: f64,
    scroll: f64,
    ring_threshold: f64,
    scroll_sign: f64,
    tap: bool,
    tap_timeout: Duration,
    tap_move_threshold: i32,
    swipe_enabled: bool,
    swipe_distance: i32,
    swipe_left: Shortcut,
    swipe_right: Shortcut,
    native_gestures: bool,
}

impl From<&Config> for Settings {
    fn from(config: &Config) -> Self {
        Self {
            pointer: config.pointer,
            scroll: config.scroll,
            ring_threshold: MAX_RADIUS * config.ring,
            scroll_sign: if config.invert_scroll { 1.0 } else { -1.0 },
            tap: config.tap,
            tap_timeout: Duration::from_millis(config.tap_timeout_ms),
            tap_move_threshold: config.tap_move_threshold,
            swipe_enabled: config.two_finger_swipe.enabled,
            swipe_distance: config.two_finger_swipe.distance,
            swipe_left: config.two_finger_swipe.left.clone(),
            swipe_right: config.two_finger_swipe.right.clone(),
            native_gestures: config.native_gestures.enabled,
        }
    }
}

pub struct DriverState {
    settings: Settings,
    slots: [SlotState; SLOT_COUNT],
    current_slot: usize,
    tap_states: [TapState; SLOT_COUNT],
    locked_zone: Option<Zone>,
    prev_angle: Option<f64>,
    prev_x: Option<i32>,
    prev_y: Option<i32>,
    scroll_accumulator: f64,
    detent_carry: i32,
    left_down: bool,
    right_down: bool,
    middle_down: bool,
    swipe_state: SwipeState,
}

impl DriverState {
    pub fn new(config: &Config) -> Self {
        Self {
            settings: Settings::from(config),
            slots: [SlotState::default(); SLOT_COUNT],
            current_slot: 0,
            tap_states: std::array::from_fn(|_| TapState::default()),
            locked_zone: None,
            prev_angle: None,
            prev_x: None,
            prev_y: None,
            scroll_accumulator: 0.0,
            detent_carry: 0,
            left_down: false,
            right_down: false,
            middle_down: false,
            swipe_state: SwipeState::Idle,
        }
    }

    pub fn process(&mut self, event: Event, now: Instant) -> Vec<Output> {
        match event {
            Event::SelectSlot(slot) => {
                self.current_slot = slot;
                Vec::new()
            }
            Event::TrackingId(id) => self.handle_tracking(id, now),
            Event::PositionX(x) => {
                if let Some(slot) = self.slots.get_mut(self.current_slot) {
                    slot.x = x;
                }
                Vec::new()
            }
            Event::PositionY(y) => {
                if let Some(slot) = self.slots.get_mut(self.current_slot) {
                    slot.y = y;
                }
                Vec::new()
            }
            Event::Button(button, value) => self.handle_button(button, value),
            Event::Frame => self.handle_frame(),
        }
    }

    fn handle_tracking(&mut self, id: i32, now: Instant) -> Vec<Output> {
        if self.current_slot >= SLOT_COUNT {
            return Vec::new();
        }

        self.slots[self.current_slot].tracking_id = id;
        if id != -1 {
            self.tap_states[self.current_slot] = TapState {
                start_time: Some(now),
                canceled: self.any_button_down(),
                ..TapState::default()
            };
            return Vec::new();
        }

        let mut outputs = Vec::new();
        if self.settings.tap {
            if self.current_slot == 0 {
                let slot0_valid = self.tap_states[0].valid(now, self.settings.tap_timeout)
                    && self.locked_zone == Some(Zone::Inner);
                let slot1_valid = self.tap_states[1].valid(now, self.settings.tap_timeout);
                let slot1_down = self.slots[1].tracking_id != -1;
                if slot1_down && slot0_valid && slot1_valid {
                    outputs.extend(click(PhysicalButton::Right));
                } else if slot0_valid && !slot1_down {
                    outputs.extend(click(PhysicalButton::Left));
                }
            } else if self.current_slot == 1 {
                let slot0_down = self.slots[0].tracking_id != -1;
                let slot0_valid = self.tap_states[0].valid(now, self.settings.tap_timeout)
                    && self.locked_zone == Some(Zone::Inner);
                let slot1_valid = self.tap_states[1].valid(now, self.settings.tap_timeout);
                if slot0_down && slot0_valid && slot1_valid {
                    outputs.extend(click(PhysicalButton::Right));
                    self.tap_states[0] = TapState::default();
                }
            }
        }

        self.tap_states[self.current_slot] = TapState::default();
        if self.current_slot == 0 {
            self.reset_primary_touch();
            self.tap_states[1] = TapState::default();
        }
        outputs
    }

    fn handle_button(&mut self, button: PhysicalButton, value: i32) -> Vec<Output> {
        let mut outputs = Vec::new();
        if value == 0 || value == 1 {
            let down = value == 1;
            match button {
                PhysicalButton::Left => self.left_down = down,
                PhysicalButton::Right => self.right_down = down,
                PhysicalButton::Middle => self.middle_down = down,
            }

            if down {
                for tap in &mut self.tap_states {
                    if tap.start_time.is_some() {
                        tap.canceled = true;
                    }
                }
                if self.swipe_state.native_active() {
                    outputs.push(Output::NativeFrame(Vec::new()));
                }
                if !matches!(&self.swipe_state, SwipeState::Idle)
                    || self.active_count() >= 2
                {
                    self.swipe_state = SwipeState::Blocked;
                    self.reset_motion_history();
                }
            }
        }

        outputs.push(Output::PointerFrame(vec![PointerEvent::Button(
            button, value,
        )]));
        outputs
    }

    fn handle_frame(&mut self) -> Vec<Output> {
        self.update_tap_movement();
        let active_count = self.active_count();

        if self.locked_zone.is_none() && self.slots[0].tracking_id != -1 {
            self.locked_zone = Some(
                classify(
                    self.slots[0].x as f64,
                    self.slots[0].y as f64,
                    self.settings.ring_threshold,
                )
                .2,
            );
        }

        let swipe_outputs = self.update_swipe_state(active_count);
        if active_count == 0 || !matches!(&self.swipe_state, SwipeState::Idle) {
            self.reset_motion_history();
            return swipe_outputs;
        }

        let slot = self.slots[0];
        if slot.tracking_id == -1 {
            return swipe_outputs;
        }

        let (_, angle, current_zone) =
            classify(slot.x as f64, slot.y as f64, self.settings.ring_threshold);
        let zone = *self.locked_zone.get_or_insert(current_zone);
        let mut pointer_events = Vec::new();

        match zone {
            Zone::Ring => {
                if let Some(previous_angle) = self.prev_angle {
                    let delta = angle_delta(previous_angle, angle);
                    self.scroll_accumulator +=
                        delta * self.settings.scroll * 120.0 * self.settings.scroll_sign;
                    let hires = self.scroll_accumulator.trunc() as i32;
                    if hires != 0 {
                        self.scroll_accumulator -= hires as f64;
                        pointer_events.push(PointerEvent::WheelHiRes(hires));
                        self.detent_carry += hires;
                        let detents = self.detent_carry / 120;
                        if detents != 0 {
                            self.detent_carry -= detents * 120;
                            pointer_events.push(PointerEvent::Wheel(detents));
                        }
                    }
                }
                self.prev_angle = Some(angle);
            }
            Zone::Inner => {
                if let (Some(previous_x), Some(previous_y)) = (self.prev_x, self.prev_y) {
                    let dx = ((slot.x - previous_x) as f64 * self.settings.pointer) as i32;
                    let dy = ((slot.y - previous_y) as f64 * self.settings.pointer) as i32;
                    if dx != 0 {
                        pointer_events.push(PointerEvent::RelX(dx));
                    }
                    if dy != 0 {
                        pointer_events.push(PointerEvent::RelY(dy));
                    }
                }
                self.prev_x = Some(slot.x);
                self.prev_y = Some(slot.y);
            }
        }

        if pointer_events.is_empty() {
            swipe_outputs
        } else {
            let mut outputs = swipe_outputs;
            outputs.push(Output::PointerFrame(pointer_events));
            outputs
        }
    }

    fn update_swipe_state(&mut self, active_count: usize) -> Vec<Output> {
        let state = std::mem::replace(&mut self.swipe_state, SwipeState::Idle);
        let mut outputs = Vec::new();

        if active_count == 0 {
            if state.native_active() {
                outputs.push(Output::NativeFrame(Vec::new()));
            }
            self.swipe_state = SwipeState::Idle;
            return outputs;
        }

        if (self.any_button_down()
            && (!matches!(&state, SwipeState::Idle) || active_count >= 2))
            || active_count >= 5
        {
            if state.native_active() {
                outputs.push(Output::NativeFrame(Vec::new()));
            }
            self.cancel_all_taps();
            self.swipe_state = SwipeState::Blocked;
            return outputs;
        }

        match state {
            SwipeState::Idle => {
                if active_count == 3 || active_count == 4 {
                    self.start_native_multi(&mut outputs);
                } else if active_count == 2 && self.two_finger_can_start() {
                    self.swipe_state = SwipeState::Two {
                        start: [
                            (self.slots[0].x, self.slots[0].y),
                            (self.slots[1].x, self.slots[1].y),
                        ],
                        direction: None,
                        horizontal_triggered: false,
                    };
                } else {
                    self.swipe_state = SwipeState::Idle;
                }
            }
            SwipeState::Blocked => {
                self.swipe_state = SwipeState::Blocked;
            }
            SwipeState::NativeTwo { synthetic_offset } => {
                if self.slots[0].tracking_id == -1 || self.slots[1].tracking_id == -1 {
                    outputs.push(Output::NativeFrame(Vec::new()));
                    self.swipe_state = SwipeState::Blocked;
                } else {
                    outputs.push(Output::NativeFrame(native_two_contacts(
                        [
                            (self.slots[0].x, self.slots[0].y),
                            (self.slots[1].x, self.slots[1].y),
                        ],
                        synthetic_offset,
                    )));
                    self.swipe_state = SwipeState::NativeTwo { synthetic_offset };
                }
            }
            SwipeState::NativeMulti { mut mapping } => {
                if active_count == 3 || active_count == 4 {
                    outputs.push(Output::NativeFrame(multi_contacts(
                        &self.slots,
                        &mut mapping,
                    )));
                    self.swipe_state = SwipeState::NativeMulti { mapping };
                } else {
                    outputs.push(Output::NativeFrame(Vec::new()));
                    self.swipe_state = SwipeState::Blocked;
                }
            }
            SwipeState::Two {
                start,
                mut direction,
                horizontal_triggered,
            } => {
                if horizontal_triggered {
                    self.swipe_state = SwipeState::Two {
                        start,
                        direction,
                        horizontal_triggered,
                    };
                } else if active_count == 3 || active_count == 4 {
                    self.start_native_multi(&mut outputs);
                } else if active_count != 2
                    || self.slots[0].tracking_id == -1
                    || self.slots[1].tracking_id == -1
                {
                    self.swipe_state = SwipeState::Blocked;
                } else {
                    let current = [
                        (self.slots[0].x, self.slots[0].y),
                        (self.slots[1].x, self.slots[1].y),
                    ];
                    if direction.is_none() {
                        direction =
                            lock_direction(start, current, self.settings.tap_move_threshold);
                        if direction.is_some() {
                            self.cancel_first_two_taps();
                        }
                    }

                    match direction {
                        Some(LockedDirection::Vertical) if self.settings.native_gestures => {
                            let start_centroid_x = (start[0].0 + start[1].0) / 2;
                            let synthetic_offset =
                                if start_centroid_x + SYNTHETIC_CONTACT_OFFSET <= PAD_MAX {
                                    SYNTHETIC_CONTACT_OFFSET
                                } else {
                                    -SYNTHETIC_CONTACT_OFFSET
                                };
                            outputs.push(Output::NativeFrame(native_two_contacts(
                                start,
                                synthetic_offset,
                            )));
                            outputs.push(Output::NativeFrame(native_two_contacts(
                                current,
                                synthetic_offset,
                            )));
                            self.swipe_state = SwipeState::NativeTwo { synthetic_offset };
                        }
                        Some(LockedDirection::Left)
                            if horizontal_reached(
                                start,
                                current,
                                self.settings.swipe_distance,
                                -1,
                            ) =>
                        {
                            outputs.push(Output::Shortcut(self.settings.swipe_left.clone()));
                            self.swipe_state = SwipeState::Two {
                                start,
                                direction,
                                horizontal_triggered: true,
                            };
                        }
                        Some(LockedDirection::Right)
                            if horizontal_reached(
                                start,
                                current,
                                self.settings.swipe_distance,
                                1,
                            ) =>
                        {
                            outputs.push(Output::Shortcut(self.settings.swipe_right.clone()));
                            self.swipe_state = SwipeState::Two {
                                start,
                                direction,
                                horizontal_triggered: true,
                            };
                        }
                        _ => {
                            self.swipe_state = SwipeState::Two {
                                start,
                                direction,
                                horizontal_triggered: false,
                            };
                        }
                    }
                }
            }
        }
        outputs
    }

    fn start_native_multi(&mut self, outputs: &mut Vec<Output>) {
        self.cancel_all_taps();
        if self.settings.native_gestures {
            let mut mapping = [None; NATIVE_SLOT_COUNT];
            outputs.push(Output::NativeFrame(multi_contacts(
                &self.slots,
                &mut mapping,
            )));
            self.swipe_state = SwipeState::NativeMulti { mapping };
        } else {
            self.swipe_state = SwipeState::Blocked;
        }
    }

    fn two_finger_can_start(&self) -> bool {
        if !self.settings.swipe_enabled
            || self.slots[0].tracking_id == -1
            || self.slots[1].tracking_id == -1
        {
            return false;
        }
        let slot0_zone = self.locked_zone.unwrap_or(Zone::Inner);
        let slot1_zone = classify(
            self.slots[1].x as f64,
            self.slots[1].y as f64,
            self.settings.ring_threshold,
        )
        .2;
        slot0_zone == Zone::Inner && slot1_zone == Zone::Inner
    }

    fn update_tap_movement(&mut self) {
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.tracking_id == -1 {
                continue;
            }
            let tap = &mut self.tap_states[index];
            if tap.start_time.is_some() && tap.start_pos.is_none() {
                tap.start_pos = Some((slot.x, slot.y));
            }
            if let Some((start_x, start_y)) = tap.start_pos {
                if (slot.x - start_x).abs() > self.settings.tap_move_threshold
                    || (slot.y - start_y).abs() > self.settings.tap_move_threshold
                {
                    tap.moved = true;
                }
            }
        }
    }

    fn active_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.tracking_id != -1)
            .count()
    }

    fn any_button_down(&self) -> bool {
        self.left_down || self.right_down || self.middle_down
    }

    fn cancel_first_two_taps(&mut self) {
        for tap in &mut self.tap_states[0..2] {
            tap.canceled = true;
        }
    }

    fn cancel_all_taps(&mut self) {
        for tap in &mut self.tap_states {
            tap.canceled = true;
        }
    }

    fn reset_motion_history(&mut self) {
        self.prev_x = None;
        self.prev_y = None;
        self.prev_angle = None;
        self.scroll_accumulator = 0.0;
        self.detent_carry = 0;
    }

    fn reset_primary_touch(&mut self) {
        self.locked_zone = None;
        self.reset_motion_history();
    }
}

fn classify(x: f64, y: f64, ring_threshold: f64) -> (f64, f64, Zone) {
    let dx = x - CENTER_X;
    let dy = y - CENTER_Y;
    let radius = (dx * dx + dy * dy).sqrt();
    let angle = dy.atan2(dx);
    let zone = if radius > ring_threshold {
        Zone::Ring
    } else {
        Zone::Inner
    };
    (radius, angle, zone)
}

fn angle_delta(previous: f64, current: f64) -> f64 {
    let mut delta = current - previous;
    if delta > PI {
        delta -= 2.0 * PI;
    } else if delta < -PI {
        delta += 2.0 * PI;
    }
    delta
}

fn lock_direction(
    start: [(i32, i32); 2],
    current: [(i32, i32); 2],
    threshold: i32,
) -> Option<LockedDirection> {
    let dx = [
        current[0].0 - start[0].0,
        current[1].0 - start[1].0,
    ];
    let dy = [
        current[0].1 - start[0].1,
        current[1].1 - start[1].1,
    ];
    let centroid_dx = (dx[0] + dx[1]) / 2;
    let centroid_dy = (dy[0] + dy[1]) / 2;
    let minimum_each = threshold / 2;

    if centroid_dx.abs() > threshold
        && centroid_dx.abs() > centroid_dy.abs()
        && dx.iter().all(|value| value.signum() == centroid_dx.signum())
        && dx.iter().all(|value| value.abs() >= minimum_each)
    {
        return Some(if centroid_dx < 0 {
            LockedDirection::Left
        } else {
            LockedDirection::Right
        });
    }
    if centroid_dy.abs() > threshold
        && centroid_dy.abs() > centroid_dx.abs()
        && dy.iter().all(|value| value.signum() == centroid_dy.signum())
        && dy.iter().all(|value| value.abs() >= minimum_each)
    {
        return Some(LockedDirection::Vertical);
    }
    None
}

fn horizontal_reached(
    start: [(i32, i32); 2],
    current: [(i32, i32); 2],
    distance: i32,
    sign: i32,
) -> bool {
    let dx0 = current[0].0 - start[0].0;
    let dx1 = current[1].0 - start[1].0;
    let centroid_dx = (dx0 + dx1) / 2;
    centroid_dx * sign >= distance
        && dx0 * sign >= distance / 2
        && dx1 * sign >= distance / 2
}

fn native_two_contacts(
    positions: [(i32, i32); 2],
    synthetic_offset: i32,
) -> Vec<NativeContact> {
    let centroid_x = (positions[0].0 + positions[1].0) / 2;
    let centroid_y = (positions[0].1 + positions[1].1) / 2;
    vec![
        NativeContact {
            slot: 0,
            tracking_id: 1,
            x: positions[0].0,
            y: positions[0].1,
        },
        NativeContact {
            slot: 1,
            tracking_id: 2,
            x: positions[1].0,
            y: positions[1].1,
        },
        NativeContact {
            slot: 2,
            tracking_id: 3,
            x: (centroid_x + synthetic_offset).clamp(0, PAD_MAX),
            y: centroid_y.clamp(0, PAD_MAX),
        },
    ]
}

fn multi_contacts(
    slots: &[SlotState; SLOT_COUNT],
    mapping: &mut [Option<i32>; NATIVE_SLOT_COUNT],
) -> Vec<NativeContact> {
    let active: Vec<_> = slots
        .iter()
        .filter(|slot| slot.tracking_id != -1)
        .copied()
        .collect();

    for mapped in mapping.iter_mut() {
        if mapped.is_some_and(|id| !active.iter().any(|slot| slot.tracking_id == id)) {
            *mapped = None;
        }
    }
    for slot in &active {
        if mapping
            .iter()
            .all(|mapped| *mapped != Some(slot.tracking_id))
        {
            if let Some(empty) = mapping.iter_mut().find(|mapped| mapped.is_none()) {
                *empty = Some(slot.tracking_id);
            }
        }
    }

    mapping
        .iter()
        .enumerate()
        .filter_map(|(virtual_slot, tracking_id)| {
            let physical = active
                .iter()
                .find(|slot| Some(slot.tracking_id) == *tracking_id)?;
            Some(NativeContact {
                slot: virtual_slot,
                tracking_id: physical.tracking_id,
                x: physical.x,
                y: physical.y,
            })
        })
        .collect()
}

fn click(button: PhysicalButton) -> Vec<Output> {
    vec![
        Output::PointerFrame(vec![PointerEvent::Button(button, 1)]),
        Output::PointerFrame(vec![PointerEvent::Button(button, 0)]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{load_from_path, Cli};
    use clap::Parser;
    use evdev::KeyCode;
    use std::path::Path;

    fn config(extra_args: &[&str]) -> Config {
        let mut args = vec!["circulartrackpad"];
        args.extend_from_slice(extra_args);
        load_from_path(&Cli::parse_from(args), Path::new("/missing")).unwrap()
    }

    fn config_with(contents: &str) -> Config {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "circulartrackpad-gestures-{}-{unique}.toml",
            std::process::id(),
        ));
        std::fs::write(&path, contents).unwrap();
        let result = load_from_path(&Cli::parse_from(["circulartrackpad"]), &path).unwrap();
        std::fs::remove_file(path).unwrap();
        result
    }

    fn now() -> Instant {
        Instant::now()
    }

    fn touch(
        state: &mut DriverState,
        slot: usize,
        tracking_id: i32,
        x: i32,
        y: i32,
        time: Instant,
    ) {
        state.process(Event::SelectSlot(slot), time);
        state.process(Event::TrackingId(tracking_id), time);
        state.process(Event::PositionX(x), time);
        state.process(Event::PositionY(y), time);
    }

    fn move_slot(state: &mut DriverState, slot: usize, x: i32, y: i32, time: Instant) {
        state.process(Event::SelectSlot(slot), time);
        state.process(Event::PositionX(x), time);
        state.process(Event::PositionY(y), time);
    }

    fn lift(state: &mut DriverState, slot: usize, time: Instant) -> Vec<Output> {
        state.process(Event::SelectSlot(slot), time);
        state.process(Event::TrackingId(-1), time)
    }

    fn begin_two(state: &mut DriverState, time: Instant) {
        touch(state, 0, 10, 220, 300, time);
        touch(state, 1, 11, 300, 300, time);
        assert!(state.process(Event::Frame, time).is_empty());
    }

    #[test]
    fn physical_buttons_always_pass_through() {
        for args in [&[][..], &["--no-tap"][..]] {
            let mut state = DriverState::new(&config(args));
            let time = now();
            for button in [
                PhysicalButton::Left,
                PhysicalButton::Right,
                PhysicalButton::Middle,
            ] {
                assert_eq!(
                    state.process(Event::Button(button, 1), time),
                    vec![Output::PointerFrame(vec![PointerEvent::Button(button, 1)])]
                );
                assert_eq!(
                    state.process(Event::Button(button, 0), time),
                    vec![Output::PointerFrame(vec![PointerEvent::Button(button, 0)])]
                );
            }
        }
    }

    #[test]
    fn button_cancels_tap_without_being_swallowed() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 264, 264, time);
        state.process(Event::Frame, time);
        assert_eq!(
            state.process(Event::Button(PhysicalButton::Left, 1), time),
            vec![Output::PointerFrame(vec![PointerEvent::Button(
                PhysicalButton::Left,
                1
            )])]
        );
        assert!(lift(&mut state, 0, time).is_empty());
    }

    #[test]
    fn single_and_two_finger_taps_are_preserved() {
        let time = now();
        let mut single = DriverState::new(&config(&[]));
        touch(&mut single, 0, 10, 264, 264, time);
        single.process(Event::Frame, time);
        assert_eq!(
            lift(&mut single, 0, time),
            click(PhysicalButton::Left)
        );

        let mut two = DriverState::new(&config(&[]));
        begin_two(&mut two, time);
        assert_eq!(lift(&mut two, 1, time), click(PhysicalButton::Right));
    }

    #[test]
    fn swipe_left_and_right_cycle_windows_once() {
        let time = now();
        let mut left = DriverState::new(&config(&[]));
        begin_two(&mut left, time);
        move_slot(&mut left, 0, 120, 300, time);
        move_slot(&mut left, 1, 200, 300, time);
        assert_eq!(
            left.process(Event::Frame, time),
            vec![Output::Shortcut(Shortcut(vec![
                KeyCode::KEY_LEFTALT,
                KeyCode::KEY_ESC
            ]))]
        );
        assert!(left.process(Event::Frame, time).is_empty());

        let mut right = DriverState::new(&config(&[]));
        begin_two(&mut right, time);
        move_slot(&mut right, 0, 320, 300, time);
        move_slot(&mut right, 1, 400, 300, time);
        assert_eq!(
            right.process(Event::Frame, time),
            vec![Output::Shortcut(Shortcut(vec![
                KeyCode::KEY_LEFTSHIFT,
                KeyCode::KEY_LEFTALT,
                KeyCode::KEY_ESC
            ]))]
        );
    }

    #[test]
    fn horizontal_threshold_and_diagonal_motion_do_not_misfire() {
        let time = now();
        let mut below = DriverState::new(&config(&[]));
        begin_two(&mut below, time);
        move_slot(&mut below, 0, 150, 300, time);
        move_slot(&mut below, 1, 230, 300, time);
        assert!(below.process(Event::Frame, time).is_empty());

        let mut diagonal = DriverState::new(&config(&[]));
        begin_two(&mut diagonal, time);
        move_slot(&mut diagonal, 0, 120, 200, time);
        move_slot(&mut diagonal, 1, 200, 200, time);
        assert!(diagonal.process(Event::Frame, time).is_empty());
    }

    #[test]
    fn vertical_two_finger_swipe_starts_and_updates_native_three_contacts() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        begin_two(&mut state, time);
        move_slot(&mut state, 0, 220, 270, time);
        move_slot(&mut state, 1, 300, 270, time);
        let start = state.process(Event::Frame, time);
        assert_eq!(start.len(), 2);
        assert!(start.iter().all(|output| {
            matches!(output, Output::NativeFrame(contacts) if contacts.len() == 3)
        }));

        move_slot(&mut state, 0, 220, 240, time);
        move_slot(&mut state, 1, 300, 240, time);
        assert!(matches!(
            state.process(Event::Frame, time).as_slice(),
            [Output::NativeFrame(contacts)] if contacts.len() == 3
                && contacts[2].y == 240
        ));

        lift(&mut state, 1, time);
        assert_eq!(
            state.process(Event::Frame, time),
            vec![Output::NativeFrame(Vec::new())]
        );
    }

    #[test]
    fn downward_two_finger_swipe_uses_the_same_native_stream() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        begin_two(&mut state, time);
        move_slot(&mut state, 0, 220, 330, time);
        move_slot(&mut state, 1, 300, 330, time);
        assert!(matches!(
            state.process(Event::Frame, time).as_slice(),
            [Output::NativeFrame(start), Output::NativeFrame(current)]
                if start.len() == 3 && current.len() == 3 && current[2].y == 330
        ));

        assert_eq!(
            state.process(Event::Button(PhysicalButton::Right, 1), time),
            vec![
                Output::NativeFrame(Vec::new()),
                Output::PointerFrame(vec![PointerEvent::Button(
                    PhysicalButton::Right,
                    1
                )])
            ]
        );
    }

    #[test]
    fn synthetic_contact_offset_is_stable_and_clamped() {
        let right = native_two_contacts([(500, 200), (528, 240)], -48);
        assert_eq!(right[2].x, 466);
        let left = native_two_contacts([(0, 200), (20, 240)], 48);
        assert_eq!(left[2].x, 58);
    }

    #[test]
    fn three_and_four_fingers_forward_native_contacts_with_stable_slots() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 120, 200, time);
        touch(&mut state, 2, 12, 260, 200, time);
        touch(&mut state, 4, 14, 400, 200, time);
        let first = state.process(Event::Frame, time);
        assert!(matches!(
            first.as_slice(),
            [Output::NativeFrame(contacts)] if contacts.len() == 3
        ));

        touch(&mut state, 1, 11, 190, 200, time);
        let four = state.process(Event::Frame, time);
        assert!(matches!(
            four.as_slice(),
            [Output::NativeFrame(contacts)] if contacts.len() == 4
                && contacts.iter().map(|contact| contact.slot).collect::<Vec<_>>()
                    == vec![0, 1, 2, 3]
        ));
    }

    #[test]
    fn third_finger_promotes_uncommitted_two_finger_sequence() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        begin_two(&mut state, time);
        touch(&mut state, 2, 12, 260, 240, time);
        assert!(matches!(
            state.process(Event::Frame, time).as_slice(),
            [Output::NativeFrame(contacts)] if contacts.len() == 3
        ));
    }

    #[test]
    fn native_multi_ends_below_three_and_five_fingers_block() {
        let time = now();
        let mut state = DriverState::new(&config(&[]));
        for slot in 0..3 {
            touch(
                &mut state,
                slot,
                slot as i32 + 10,
                180 + slot as i32 * 60,
                240,
                time,
            );
        }
        state.process(Event::Frame, time);
        lift(&mut state, 2, time);
        assert_eq!(
            state.process(Event::Frame, time),
            vec![Output::NativeFrame(Vec::new())]
        );

        let mut five = DriverState::new(&config(&[]));
        for slot in 0..3 {
            touch(
                &mut five,
                slot,
                slot as i32 + 20,
                100 + slot as i32 * 70,
                240,
                time,
            );
        }
        assert!(matches!(
            five.process(Event::Frame, time).as_slice(),
            [Output::NativeFrame(contacts)] if contacts.len() == 3
        ));
        for slot in 3..5 {
            touch(
                &mut five,
                slot,
                slot as i32 + 20,
                100 + slot as i32 * 70,
                240,
                time,
            );
        }
        assert_eq!(
            five.process(Event::Frame, time),
            vec![Output::NativeFrame(Vec::new())]
        );
        move_slot(&mut five, 0, 50, 200, time);
        assert!(five.process(Event::Frame, time).is_empty());
    }

    #[test]
    fn native_backend_can_be_disabled_without_disabling_horizontal_swipes() {
        let mut state =
            DriverState::new(&config_with("[native_gestures]\nenabled = false\n"));
        let time = now();
        begin_two(&mut state, time);
        move_slot(&mut state, 0, 120, 300, time);
        move_slot(&mut state, 1, 200, 300, time);
        assert!(matches!(
            state.process(Event::Frame, time).as_slice(),
            [Output::Shortcut(_)]
        ));

        let mut vertical =
            DriverState::new(&config_with("[native_gestures]\nenabled = false\n"));
        begin_two(&mut vertical, time);
        move_slot(&mut vertical, 0, 220, 200, time);
        move_slot(&mut vertical, 1, 300, 200, time);
        assert!(vertical.process(Event::Frame, time).is_empty());
    }

    #[test]
    fn single_finger_pointer_motion_and_ring_scroll_are_preserved() {
        let time = now();
        let mut pointer = DriverState::new(&config(&[]));
        touch(&mut pointer, 0, 10, 264, 264, time);
        pointer.process(Event::Frame, time);
        move_slot(&mut pointer, 0, 274, 259, time);
        assert_eq!(
            pointer.process(Event::Frame, time),
            vec![Output::PointerFrame(vec![
                PointerEvent::RelX(15),
                PointerEvent::RelY(-7)
            ])]
        );

        let mut ring = DriverState::new(&config(&[]));
        ring.process(Event::Button(PhysicalButton::Left, 1), time);
        touch(&mut ring, 0, 10, 464, 264, time);
        ring.process(Event::Frame, time);
        move_slot(&mut ring, 0, 436, 367, time);
        assert!(matches!(
            ring.process(Event::Frame, time).as_slice(),
            [Output::PointerFrame(events)]
                if events.iter().any(|event| matches!(event, PointerEvent::WheelHiRes(_)))
        ));
    }

    #[test]
    fn angle_delta_wraps_in_both_directions() {
        assert!((angle_delta(PI - 0.1, -PI + 0.1) - 0.2).abs() < 1e-9);
        assert!((angle_delta(-PI + 0.1, PI - 0.1) + 0.2).abs() < 1e-9);
    }
}
