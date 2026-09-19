use evdev::uinput::VirtualDevice;
use evdev::{AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, InputEvent as EvdevEvent, KeyCode, PropType, RelativeAxisCode, UinputAbsSetup};
use std::collections::HashSet;

use crate::{InputEmitter, PlatformError, Result};
use smkb_proto::{DisplayGeometry, InputEvent};

const MOUSE_BUTTONS: [KeyCode; 8] = [
    KeyCode::BTN_LEFT,
    KeyCode::BTN_RIGHT,
    KeyCode::BTN_MIDDLE,
    KeyCode::BTN_SIDE,
    KeyCode::BTN_EXTRA,
    KeyCode::BTN_FORWARD,
    KeyCode::BTN_BACK,
    KeyCode::BTN_TASK,
];

const KEY_RANGE: std::ops::Range<u16> = 1..0x2ff;

fn is_keyboard_key(code: u16) -> bool {
    format!("{:?}", KeyCode(code)).starts_with("KEY_")
}

fn abs_event(code: AbsoluteAxisCode, value: i32) -> EvdevEvent {
    EvdevEvent::new(EventType::ABSOLUTE.0, code.0, value)
}

fn key_event(code: u16, pressed: bool) -> EvdevEvent {
    EvdevEvent::new(EventType::KEY.0, code, i32::from(pressed))
}

fn rel_event(code: RelativeAxisCode, value: i32) -> EvdevEvent {
    EvdevEvent::new(EventType::RELATIVE.0, code.0, value)
}

pub struct UinputEmitter {
    pointer: VirtualDevice,
    keyboard: VirtualDevice,
    held_keys: HashSet<u16>,
    held_buttons: HashSet<u16>,
}

impl UinputEmitter {
    pub fn new(screen: DisplayGeometry) -> Result<Self> {
        let mut buttons = AttributeSet::<KeyCode>::new();
        for code in MOUSE_BUTTONS {
            buttons.insert(code);
        }
        let mut rel_axes = AttributeSet::<RelativeAxisCode>::new();
        for axis in [
            RelativeAxisCode::REL_WHEEL,
            RelativeAxisCode::REL_HWHEEL,
            RelativeAxisCode::REL_WHEEL_HI_RES,
            RelativeAxisCode::REL_HWHEEL_HI_RES,
        ] {
            rel_axes.insert(axis);
        }
        let mut props = AttributeSet::<PropType>::new();
        props.insert(PropType::POINTER);

        let abs_x = UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, AbsInfo::new(0, 0, screen.width.saturating_sub(1) as i32, 0, 0, 0));
        let abs_y = UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, AbsInfo::new(0, 0, screen.height.saturating_sub(1) as i32, 0, 0, 0));

        let pointer = VirtualDevice::builder()
            .map_err(PlatformError::Io)?
            .name("smkb-virtual-pointer")
            .with_keys(&buttons)
            .map_err(PlatformError::Io)?
            .with_absolute_axis(&abs_x)
            .map_err(PlatformError::Io)?
            .with_absolute_axis(&abs_y)
            .map_err(PlatformError::Io)?
            .with_relative_axes(&rel_axes)
            .map_err(PlatformError::Io)?
            .with_properties(&props)
            .map_err(PlatformError::Io)?
            .build()
            .map_err(PlatformError::Io)?;

        let mut keys = AttributeSet::<KeyCode>::new();
        for code in KEY_RANGE.filter(|&c| is_keyboard_key(c)) {
            keys.insert(KeyCode(code));
        }
        let keyboard = VirtualDevice::builder()
            .map_err(PlatformError::Io)?
            .name("smkb-virtual-keyboard")
            .with_keys(&keys)
            .map_err(PlatformError::Io)?
            .build()
            .map_err(PlatformError::Io)?;

        Ok(Self { pointer, keyboard, held_keys: HashSet::new(), held_buttons: HashSet::new() })
    }
}

impl InputEmitter for UinputEmitter {
    fn emit(&mut self, event: &InputEvent) -> Result<()> {
        match event {
            InputEvent::Motion { x, y } => self
                .pointer
                .emit(&[abs_event(AbsoluteAxisCode::ABS_X, *x), abs_event(AbsoluteAxisCode::ABS_Y, *y)])
                .map_err(PlatformError::Io),

            InputEvent::Button { code, pressed, at } => {
                self.pointer
                    .emit(&[
                        abs_event(AbsoluteAxisCode::ABS_X, at.x),
                        abs_event(AbsoluteAxisCode::ABS_Y, at.y),
                        key_event(*code, *pressed),
                    ])
                    .map_err(PlatformError::Io)?;
                track(&mut self.held_buttons, *code, *pressed);
                Ok(())
            }

            InputEvent::Key { code, pressed } => {
                self.keyboard.emit(&[key_event(*code, *pressed)]).map_err(PlatformError::Io)?;
                track(&mut self.held_keys, *code, *pressed);
                Ok(())
            }

            InputEvent::Scroll { dx, dy, hi_res } => {
                let (x_axis, y_axis) = if *hi_res {
                    (RelativeAxisCode::REL_HWHEEL_HI_RES, RelativeAxisCode::REL_WHEEL_HI_RES)
                } else {
                    (RelativeAxisCode::REL_HWHEEL, RelativeAxisCode::REL_WHEEL)
                };
                let mut evs = Vec::with_capacity(2);
                if *dx != 0 {
                    evs.push(rel_event(x_axis, *dx));
                }
                if *dy != 0 {
                    evs.push(rel_event(y_axis, *dy));
                }
                if evs.is_empty() {
                    return Ok(());
                }
                self.pointer.emit(&evs).map_err(PlatformError::Io)
            }

            InputEvent::Enter { at } => self
                .pointer
                .emit(&[abs_event(AbsoluteAxisCode::ABS_X, at.x), abs_event(AbsoluteAxisCode::ABS_Y, at.y)])
                .map_err(PlatformError::Io),

            InputEvent::Leave => Ok(()),

            InputEvent::ReleaseAll => self.release_all(),
        }
    }

    fn release_all(&mut self) -> Result<()> {
        let key_ups: Vec<_> = self.held_keys.drain().map(|code| key_event(code, false)).collect();
        if !key_ups.is_empty() {
            self.keyboard.emit(&key_ups).map_err(PlatformError::Io)?;
        }
        let button_ups: Vec<_> = self.held_buttons.drain().map(|code| key_event(code, false)).collect();
        if !button_ups.is_empty() {
            self.pointer.emit(&button_ups).map_err(PlatformError::Io)?;
        }
        Ok(())
    }
}

fn track(held: &mut HashSet<u16>, code: u16, pressed: bool) {
    if pressed {
        held.insert(code);
    } else {
        held.remove(&code);
    }
}
