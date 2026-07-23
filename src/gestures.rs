use crate::config::{Config, Shortcut};
use std::f64::consts::PI;
use std::time::{Duration, Instant};

const PAD_MAX: f64 = 528.0;
const CENTER_X: f64 = PAD_MAX / 2.0;
const CENTER_Y: f64 = PAD_MAX / 2.0;
const MAX_RADIUS: f64 = PAD_MAX / 2.0;
const SLOT_COUNT: usize = 5;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Output {
    PointerFrame(Vec<PointerEvent>),
    Shortcut(Shortcut),
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

#[derive(Debug, Clone, Copy)]
struct SwipeCandidate {
    start: [(i32, i32); 2],
    triggered: bool,
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
    gesture_step: f64,
    left_clockwise: Shortcut,
    left_counterclockwise: Shortcut,
    right_clockwise: Shortcut,
    right_counterclockwise: Shortcut,
    swipe_enabled: bool,
    swipe_distance: i32,
    swipe_up: Shortcut,
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
            gesture_step: config.button_gestures.step_degrees.to_radians(),
            left_clockwise: config.button_gestures.left_clockwise.clone(),
            left_counterclockwise: config.button_gestures.left_counterclockwise.clone(),
            right_clockwise: config.button_gestures.right_clockwise.clone(),
            right_counterclockwise: config.button_gestures.right_counterclockwise.clone(),
            swipe_enabled: config.two_finger_swipe.enabled,
            swipe_distance: config.two_finger_swipe.distance,
            swipe_up: config.two_finger_swipe.up.clone(),
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
    gesture_accumulator: f64,
    left_down: bool,
    right_down: bool,
    swipe_candidate: Option<SwipeCandidate>,
    swipe_blocked_until_lift: bool,
    swipe_consuming_sequence: bool,
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
            gesture_accumulator: 0.0,
            left_down: false,
            right_down: false,
            swipe_candidate: None,
            swipe_blocked_until_lift: false,
            swipe_consuming_sequence: false,
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
            Event::Frame => self.handle_frame(now),
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
                canceled: self.left_down || self.right_down,
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

        if self.swipe_candidate.take().is_some() {
            self.swipe_blocked_until_lift = true;
            self.swipe_consuming_sequence = true;
            self.reset_motion_history();
        }
        if self.active_count() == 0 {
            self.reset_swipe_sequence();
        }
        outputs
    }

    fn handle_button(&mut self, button: PhysicalButton, value: i32) -> Vec<Output> {
        if button == PhysicalButton::Middle {
            return vec![Output::PointerFrame(vec![PointerEvent::Button(
                button, value,
            )])];
        }

        if value == 0 || value == 1 {
            let down = value == 1;
            match button {
                PhysicalButton::Left => self.left_down = down,
                PhysicalButton::Right => self.right_down = down,
                PhysicalButton::Middle => unreachable!(),
            }
        }

        if !self.settings.tap {
            return vec![Output::PointerFrame(vec![PointerEvent::Button(
                button, value,
            )])];
        }

        if value != 0 && value != 1 {
            return Vec::new();
        }

        let down = value == 1;
        self.gesture_accumulator = 0.0;
        self.scroll_accumulator = 0.0;
        self.detent_carry = 0;
        if down {
            for tap in &mut self.tap_states {
                if tap.start_time.is_some() {
                    tap.canceled = true;
                }
            }
            if self.swipe_candidate.take().is_some() {
                self.swipe_blocked_until_lift = true;
                self.swipe_consuming_sequence = true;
                self.reset_motion_history();
            }
        }
        Vec::new()
    }

    fn handle_frame(&mut self, _now: Instant) -> Vec<Output> {
        self.update_tap_movement();
        let active_count = self.active_count();

        if active_count == 0 {
            self.reset_swipe_sequence();
            return Vec::new();
        }

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

        self.update_swipe_state(active_count);
        if self.swipe_consuming_sequence {
            self.reset_motion_history();
            if let Some(candidate) = &mut self.swipe_candidate {
                if !candidate.triggered {
                    let start = candidate.start;
                    let current = [
                        (self.slots[0].x, self.slots[0].y),
                        (self.slots[1].x, self.slots[1].y),
                    ];
                    if swipe_triggered(start, current, self.settings.swipe_distance) {
                        candidate.triggered = true;
                        for tap in &mut self.tap_states[0..2] {
                            tap.canceled = true;
                        }
                        return vec![Output::Shortcut(self.settings.swipe_up.clone())];
                    }
                }
            }
            return Vec::new();
        }

        let slot = self.slots[0];
        if slot.tracking_id == -1 {
            return Vec::new();
        }

        let (_, angle, current_zone) =
            classify(slot.x as f64, slot.y as f64, self.settings.ring_threshold);
        let zone = *self.locked_zone.get_or_insert(current_zone);
        let mut pointer_events = Vec::new();

        match zone {
            Zone::Ring => {
                if let Some(previous_angle) = self.prev_angle {
                    let delta = angle_delta(previous_angle, angle);
                    if let Some(mode) = self.button_mode() {
                        self.gesture_accumulator += delta;
                        if self.gesture_accumulator >= self.settings.gesture_step {
                            self.gesture_accumulator -= self.settings.gesture_step;
                            return vec![Output::Shortcut(self.clockwise_shortcut(mode))];
                        }
                        if self.gesture_accumulator <= -self.settings.gesture_step {
                            self.gesture_accumulator += self.settings.gesture_step;
                            return vec![Output::Shortcut(self.counterclockwise_shortcut(mode))];
                        }
                    } else if !self.settings.tap || !(self.left_down && self.right_down) {
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
            Vec::new()
        } else {
            vec![Output::PointerFrame(pointer_events)]
        }
    }

    fn update_tap_movement(&mut self) {
        for index in 0..2 {
            let slot = self.slots[index];
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

    fn update_swipe_state(&mut self, active_count: usize) {
        if !self.settings.swipe_enabled {
            return;
        }
        if active_count > 2 {
            if self.swipe_candidate.take().is_some() {
                self.swipe_consuming_sequence = true;
                for tap in &mut self.tap_states[0..2] {
                    tap.canceled = true;
                }
            }
            self.swipe_blocked_until_lift = true;
            return;
        }
        if self.swipe_blocked_until_lift {
            return;
        }
        if active_count != 2 || self.slots[0].tracking_id == -1 || self.slots[1].tracking_id == -1 {
            return;
        }
        if self.left_down || self.right_down {
            self.swipe_blocked_until_lift = true;
            return;
        }
        if self.swipe_candidate.is_some() {
            return;
        }

        let slot0_zone = self.locked_zone.unwrap_or(Zone::Inner);
        let slot1_zone = classify(
            self.slots[1].x as f64,
            self.slots[1].y as f64,
            self.settings.ring_threshold,
        )
        .2;
        self.swipe_blocked_until_lift = true;
        if slot0_zone == Zone::Inner && slot1_zone == Zone::Inner {
            self.swipe_candidate = Some(SwipeCandidate {
                start: [
                    (self.slots[0].x, self.slots[0].y),
                    (self.slots[1].x, self.slots[1].y),
                ],
                triggered: false,
            });
            self.swipe_consuming_sequence = true;
        }
    }

    fn active_count(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| slot.tracking_id != -1)
            .count()
    }

    fn button_mode(&self) -> Option<PhysicalButton> {
        if !self.settings.tap {
            return None;
        }
        match (self.left_down, self.right_down) {
            (true, false) => Some(PhysicalButton::Left),
            (false, true) => Some(PhysicalButton::Right),
            _ => None,
        }
    }

    fn clockwise_shortcut(&self, mode: PhysicalButton) -> Shortcut {
        match mode {
            PhysicalButton::Left => self.settings.left_clockwise.clone(),
            PhysicalButton::Right => self.settings.right_clockwise.clone(),
            PhysicalButton::Middle => unreachable!(),
        }
    }

    fn counterclockwise_shortcut(&self, mode: PhysicalButton) -> Shortcut {
        match mode {
            PhysicalButton::Left => self.settings.left_counterclockwise.clone(),
            PhysicalButton::Right => self.settings.right_counterclockwise.clone(),
            PhysicalButton::Middle => unreachable!(),
        }
    }

    fn reset_motion_history(&mut self) {
        self.prev_x = None;
        self.prev_y = None;
        self.prev_angle = None;
        self.scroll_accumulator = 0.0;
        self.detent_carry = 0;
        self.gesture_accumulator = 0.0;
    }

    fn reset_primary_touch(&mut self) {
        self.locked_zone = None;
        self.reset_motion_history();
    }

    fn reset_swipe_sequence(&mut self) {
        self.swipe_candidate = None;
        self.swipe_blocked_until_lift = false;
        self.swipe_consuming_sequence = false;
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

fn swipe_triggered(start: [(i32, i32); 2], current: [(i32, i32); 2], distance: i32) -> bool {
    let dx0 = current[0].0 - start[0].0;
    let dx1 = current[1].0 - start[1].0;
    let dy0 = current[0].1 - start[0].1;
    let dy1 = current[1].1 - start[1].1;
    let centroid_dx = (dx0 + dx1) / 2;
    let centroid_dy = (dy0 + dy1) / 2;
    let upward = -centroid_dy;
    upward >= distance && -dy0 >= distance / 2 && -dy1 >= distance / 2 && upward > centroid_dx.abs()
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

    #[test]
    fn two_finger_swipe_emits_super_once_and_not_right_click() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 220, 350, time);
        touch(&mut state, 1, 11, 300, 350, time);
        state.process(Event::Frame, time);

        move_slot(&mut state, 0, 220, 250, time);
        move_slot(&mut state, 1, 300, 250, time);
        let output = state.process(Event::Frame, time);
        assert_eq!(
            output,
            vec![Output::Shortcut(Shortcut(vec![KeyCode::KEY_LEFTMETA]))]
        );
        assert!(state.process(Event::Frame, time).is_empty());

        state.process(Event::SelectSlot(1), time);
        let lift = state.process(Event::TrackingId(-1), time);
        assert!(lift.is_empty());
    }

    #[test]
    fn two_finger_tap_remains_right_click() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 240, 280, time);
        touch(&mut state, 1, 11, 290, 280, time);
        state.process(Event::Frame, time);
        state.process(Event::SelectSlot(1), time);
        assert_eq!(
            state.process(Event::TrackingId(-1), time),
            click(PhysicalButton::Right)
        );
    }

    #[test]
    fn single_finger_tap_remains_left_click() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 264, 264, time);
        state.process(Event::Frame, time);
        assert_eq!(
            state.process(Event::TrackingId(-1), time),
            click(PhysicalButton::Left)
        );
    }

    #[test]
    fn horizontal_two_finger_motion_does_not_trigger() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 200, 264, time);
        touch(&mut state, 1, 11, 280, 264, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 310, 254, time);
        move_slot(&mut state, 1, 390, 254, time);
        assert!(state.process(Event::Frame, time).is_empty());
    }

    #[test]
    fn ring_started_contacts_do_not_arm_swipe() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 500, 264, time);
        touch(&mut state, 1, 11, 300, 350, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 500, 150, time);
        move_slot(&mut state, 1, 300, 230, time);
        assert!(state
            .process(Event::Frame, time)
            .iter()
            .all(|output| !matches!(output, Output::Shortcut(_))));
    }

    #[test]
    fn left_button_ring_clockwise_switches_window_without_click_or_scroll() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        assert!(state
            .process(Event::Button(PhysicalButton::Left, 1), time)
            .is_empty());
        touch(&mut state, 0, 10, 464, 264, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 436, 367, time);
        assert_eq!(
            state.process(Event::Frame, time),
            vec![Output::Shortcut(Shortcut(vec![
                KeyCode::KEY_LEFTALT,
                KeyCode::KEY_ESC
            ]))]
        );
    }

    #[test]
    fn button_first_and_ring_first_are_both_supported() {
        let time = now();
        let mut ring_first = DriverState::new(&config(&[]));
        touch(&mut ring_first, 0, 10, 464, 264, time);
        ring_first.process(Event::Frame, time);
        ring_first.process(Event::Button(PhysicalButton::Right, 1), time);
        move_slot(&mut ring_first, 0, 436, 367, time);
        assert!(matches!(
            ring_first.process(Event::Frame, time).as_slice(),
            [Output::Shortcut(_)]
        ));
    }

    #[test]
    fn counterclockwise_ring_uses_reverse_window_shortcut() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        state.process(Event::Button(PhysicalButton::Left, 1), time);
        touch(&mut state, 0, 10, 464, 264, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 436, 161, time);
        assert_eq!(
            state.process(Event::Frame, time),
            vec![Output::Shortcut(Shortcut(vec![
                KeyCode::KEY_LEFTSHIFT,
                KeyCode::KEY_LEFTALT,
                KeyCode::KEY_ESC
            ]))]
        );
    }

    #[test]
    fn right_button_clockwise_ring_uses_next_workspace_shortcut() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        state.process(Event::Button(PhysicalButton::Right, 1), time);
        touch(&mut state, 0, 10, 464, 264, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 436, 367, time);
        assert_eq!(
            state.process(Event::Frame, time),
            vec![Output::Shortcut(Shortcut(vec![
                KeyCode::KEY_LEFTMETA,
                KeyCode::KEY_PAGEDOWN
            ]))]
        );
    }

    #[test]
    fn both_buttons_suppress_ring_navigation() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        state.process(Event::Button(PhysicalButton::Left, 1), time);
        state.process(Event::Button(PhysicalButton::Right, 1), time);
        touch(&mut state, 0, 10, 464, 264, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 436, 367, time);
        assert!(state.process(Event::Frame, time).is_empty());
        assert!(state
            .process(Event::Button(PhysicalButton::Left, 0), time)
            .is_empty());
        assert!(state
            .process(Event::Button(PhysicalButton::Right, 0), time)
            .is_empty());
    }

    #[test]
    fn button_held_when_second_finger_arrives_blocks_swipe_for_sequence() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        state.process(Event::Button(PhysicalButton::Left, 1), time);
        touch(&mut state, 0, 10, 220, 350, time);
        touch(&mut state, 1, 11, 300, 350, time);
        state.process(Event::Frame, time);
        state.process(Event::Button(PhysicalButton::Left, 0), time);
        move_slot(&mut state, 0, 220, 240, time);
        move_slot(&mut state, 1, 300, 240, time);
        assert!(state
            .process(Event::Frame, time)
            .iter()
            .all(|output| !matches!(output, Output::Shortcut(_))));
    }

    #[test]
    fn third_finger_cancels_swipe_candidate() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 210, 350, time);
        touch(&mut state, 1, 11, 270, 350, time);
        state.process(Event::Frame, time);
        touch(&mut state, 2, 12, 330, 350, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 210, 230, time);
        move_slot(&mut state, 1, 270, 230, time);
        assert!(state.process(Event::Frame, time).is_empty());
    }

    #[test]
    fn single_finger_pointer_motion_is_preserved() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 264, 264, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 274, 259, time);
        assert_eq!(
            state.process(Event::Frame, time),
            vec![Output::PointerFrame(vec![
                PointerEvent::RelX(15),
                PointerEvent::RelY(-7)
            ])]
        );
    }

    #[test]
    fn unmodified_ring_motion_still_scrolls() {
        let mut state = DriverState::new(&config(&[]));
        let time = now();
        touch(&mut state, 0, 10, 464, 264, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 436, 367, time);
        let output = state.process(Event::Frame, time);
        assert!(matches!(
            output.as_slice(),
            [Output::PointerFrame(events)]
                if events.iter().any(|event| matches!(event, PointerEvent::WheelHiRes(_)))
        ));
    }

    #[test]
    fn no_tap_restores_buttons_and_keeps_swipe() {
        let mut state = DriverState::new(&config(&["--no-tap"]));
        let time = now();
        assert_eq!(
            state.process(Event::Button(PhysicalButton::Left, 1), time),
            vec![Output::PointerFrame(vec![PointerEvent::Button(
                PhysicalButton::Left,
                1
            )])]
        );
        state.process(Event::Button(PhysicalButton::Left, 0), time);

        touch(&mut state, 0, 10, 220, 350, time);
        touch(&mut state, 1, 11, 300, 350, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 220, 250, time);
        move_slot(&mut state, 1, 300, 250, time);
        assert!(matches!(
            state.process(Event::Frame, time).as_slice(),
            [Output::Shortcut(_)]
        ));
    }

    #[test]
    fn no_tap_never_uses_physical_button_as_ring_mode() {
        let mut state = DriverState::new(&config(&["--no-tap"]));
        let time = now();
        state.process(Event::Button(PhysicalButton::Left, 1), time);
        touch(&mut state, 0, 10, 464, 264, time);
        state.process(Event::Frame, time);
        move_slot(&mut state, 0, 436, 367, time);
        assert!(state
            .process(Event::Frame, time)
            .iter()
            .all(|output| !matches!(output, Output::Shortcut(_))));
    }

    #[test]
    fn angle_delta_wraps_in_both_directions() {
        assert!((angle_delta(PI - 0.1, -PI + 0.1) - 0.2).abs() < 1e-9);
        assert!((angle_delta(-PI + 0.1, PI - 0.1) + 0.2).abs() < 1e-9);
    }
}
