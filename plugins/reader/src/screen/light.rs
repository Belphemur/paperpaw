//! The front light set by a slide along the screen's sides, at a
//! finger: starting in the band along the left edge and sliding up
//! warms the light, sliding down cools it; the band along the right
//! edge does the same for the brightness, up for brighter, all the way
//! down for off and up again for on (the feel of the CrossPoint's
//! side gestures). Only the light's setting changes, shown before it is
//! kept as a slider's is, so the light answers while the finger moves
//! and the page is not drawn again at any point.
//!
//! The settings are set at once the way the reader's panel slider does:
//! shown, not kept, until the finger is lifted; the light is then as
//! the value says, and the page as it was (the kernel redraws nothing
//! a light's change has to redraw).
//!
//! The bands are the shares of the screen's width that *Light bands at
//! the sides* says (0: none), less what the kernel keeps for itself at
//! each edge: page turns by swipe and by tap stay where they were, and
//! back and the quick panel keep the top and the bottom strips. A
//! horizontal move in a band turns the page as a swipe in from that
//! side would, so a band is no trap while reading; a tap in one does
//! what a tap there does otherwise, a held one too.
//!
//! The light's settings are those the shell puts at the front of the
//! reader's panel as slider numbers ([`menus::READER_PANEL`]), in
//! order: the first the brightness, the second the warmth, as the X4
//! Pro's, the Android's and the emulated device's light are registered
//! ([the display group's](groups::DISPLAY) sliders, from 0, so no other
//! number of the panel is mistaken for one). Where there is no second
//! one, the left band is not taken.
//!
//! Where the finger ends without it coming to the light, what runs is
//! what would run there without a band, as dispatch walks it: the
//! screen's own gestures, then the key map's where the kernel answers
//! for it (neither under its own-gestures nor its gestures-off policy),
//! the ordinary 3×3 zones of the key map (a layer's own needs asking
//! the kernel for, which no view of the key map does yet), a double tap
//! of a zone waited for, and a long press fired at its time while the
//! finger still holds. What a pinch begun in a band makes also rests a
//! platform's limit: a finger a screen has taken is the screen's, and
//! the kernel keeps no finger but its own.

use alloc::string::ToString;
use alloc::vec::Vec;

use super::*;
use paperpaw_sdk::builtin::{GLOBAL_KEYMAP_LAYER, menus};
use paperpaw_sdk::builtin::{groups, settings as core_settings};
use paperpaw_sdk::{
    Capability, Direction, Instant, PointerEvent, PointerPhase, PointerTool, Setting, SettingKind,
    SettingSpec, SettingValue,
};

/// How long a swipe in from an edge waits for its second one, on a
/// screen whose gestures are its own (the kernel's rule, kept as a
/// band's swipe goes the same way as theirs).
const EDGE_AGAIN_MS: u64 = 3_000;

/// What a finger in a band is at: the light (a slide up and down), or
/// the page (a move across, as a swipe would).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Intent {
    Light,
    Turn,
}

/// A setting of the light, found once a drag in a band comes down: its
/// id, its range and the steps its value goes in.
#[derive(Debug, Clone)]
struct Target {
    id: Id,
    min: i64,
    max: i64,
    step: i64,
}

impl Target {
    /// The light's setting `n` (0 the brightness, 1 the warmth), if the
    /// device can set it: among the shell's own, else the kernel's
    /// ([`core_settings::FRONTLIGHT`]) where it says it has a light.
    pub(super) fn of(env: &Env<'_>, n: usize) -> Option<Self> {
        let spec = match n {
            0 => Self::shell(env, 0).or_else(|| {
                env.caps()
                    .frontlight
                    .then(|| core_settings::FRONTLIGHT.clone())
                    .and_then(|setting| env.settings().spec(setting.as_str()).cloned())
            }),
            1 => Self::shell(env, 1).or_else(|| {
                (env.caps().frontlight && env.caps().has(Capability::WARMTH))
                    .then(|| core_settings::FRONTLIGHT_WARMTH.clone())
                    .and_then(|setting| env.settings().spec(setting.as_str()).cloned())
            }),
            _ => None,
        }?;
        let SettingKind::Int { min, max } = spec.kind else {
            return None;
        };
        Some(Self {
            id: spec.id.clone(),
            min,
            max,
            step: spec.extra().step.max(1),
        })
    }

    /// The shell's light-setting `n`: the slider numbers of the display
    /// group (from `min` 0) among the reader's panel's entries, in the
    /// order they show, as the shells register their light in: the first
    /// the brightness, the second the warmth where the device has one
    /// (`shells/desktop`, `xteink` and `android` each put their light's
    /// sliders at the panel's front, in the display group). No other
    /// slider is a light's: the panel's every number is shown there, and
    /// the reader's them (the font size, the spacing, the margins) come
    /// in their reading groups, so a panel the user laid out anew puts
    /// no non-light slider where a light's is.
    fn shell(env: &Env<'_>, n: usize) -> Option<SettingSpec> {
        let sliders: Vec<SettingSpec> = env
            .menus()
            .items(menus::READER_PANEL.as_str())
            .into_iter()
            .filter_map(|item| env.settings().spec(item.id.as_str()).cloned())
            .filter(|spec| {
                matches!(spec.kind, SettingKind::Int { min: 0, .. })
                    && spec.group == groups::DISPLAY
                    && spec.extra().slider
            })
            .collect();
        sliders.get(n).cloned()
    }

    /// The value kept now, within range.
    fn value(&self, env: &Env<'_>) -> i64 {
        env.settings()
            .int(self.id.as_str())
            .clamp(self.min, self.max)
    }

    /// `value` on its step (as the slider would show it), in range.
    fn aligned(&self, value: i64) -> i64 {
        if self.step.max(1) > 1 {
            let step = self.step.max(1);
            ((value + step / 2).div_euclid(step) * step).clamp(self.min, self.max)
        } else {
            value.clamp(self.min, self.max)
        }
    }

    /// The value while the finger has moved `moved` pixels up from where
    /// it came down (`from`): the light's whole range is reached over
    /// half the screen's height, so a stroke a third up covers some two
    /// thirds of it.
    fn shown(&self, env: &Env<'_>, from: i64, moved: i32) -> i64 {
        let height = i64::from(env.device().display.height.max(1));
        let raw = from + i64::from(moved) * self.max * 2 / height;
        self.aligned(raw)
    }
}

/// A finger taken along in a band.
#[derive(Debug)]
struct Drag {
    pointer: u8,
    /// Which band it came down in: left (warmth) or right (brightness).
    left: bool,
    /// What that band sets.
    target: Target,
    /// Where the finger came down, and when, and the value then.
    start: Point,
    at: Instant,
    from: i64,
    /// The value shown now (`from` until the finger moves).
    value: i64,
    /// The finger's tool, for the gestures its lift may make.
    tool: PointerTool,
    /// What the finger is at, from its first move past the dead zone on:
    /// decided once, kept while it moves.
    intent: Option<Intent>,
    /// A value has been shown without being kept meanwhile.
    shown: bool,
    /// When the long press of a finger held still is due. Kept only
    /// while nothing else of the touch is decided: the first move past
    /// the dead zone says what it is (as the kernel's recognizer stops
    /// waiting for the long press once it moves).
    hold: Option<Instant>,
    /// The long press ran at its time, while the finger still held:
    /// the finger does no more, its lift least (as the kernel's).
    fired: bool,
}

/// The light at the screen's sides: a finger held in a band, and a
/// swipe in from a side that, where the reader's gestures are the
/// screen's own, waits a few seconds for the second one.
#[derive(Debug, Default)]
pub(super) struct LightSwipe {
    drag: Option<Drag>,
    /// A side swipe that awaits its second one, as the kernel's edges
    /// wait for theirs.
    pending: Option<(Direction, Instant)>,
    /// A band tap held back for its second, where something is bound
    /// to `double-tap.<zone>`: the input the first tap would run, and
    /// when it is too late for a second one.
    tap: Option<(String, Instant)>,
    /// What a finger's drag is showing without being kept, if it is:
    /// the command to take the values back is the reader's while
    /// another screen holds its place (`Screen::on_hide` asks none).
    shown: bool,
}

/// The nine tap zones of the 3×3 grid over the screen, as key maps
/// write them: the one `point` falls in.
fn zone_of(point: Point, width: u16, height: u16) -> &'static str {
    let third = |value: i32, size: u16| {
        let size = i64::from(size.max(1));
        (i64::from(value).clamp(0, size - 1) * 3 / size) as usize
    };
    const ZONES: [&str; 9] = [
        "top-left",
        "top",
        "top-right",
        "left",
        "center",
        "right",
        "bottom-left",
        "bottom",
        "bottom-right",
    ];
    ZONES[third(point.y, height) * 3 + third(point.x, width)]
}

/// The zone on the other side, where the pages go the other way round.
fn other_zone(zone: &'static str) -> &'static str {
    match zone {
        "top-left" => "top-right",
        "top-right" => "top-left",
        "left" => "right",
        "right" => "left",
        "bottom-left" => "bottom-right",
        "bottom-right" => "bottom-left",
        other => other,
    }
}

/// The direction a finger went, where it started and where it ended.
fn swipe_direction(start: Point, end: Point) -> Direction {
    let (dx, dy) = (end.x - start.x, end.y - start.y);
    if dx.abs() >= dy.abs() {
        if dx < 0 {
            Direction::Left
        } else {
            Direction::Right
        }
    } else if dy < 0 {
        Direction::Up
    } else {
        Direction::Down
    }
}

impl LightSwipe {
    /// What a finger coming down at `point` is taken for: a band's
    /// setting, where the setting it names is.
    fn takes(&self, env: &Env<'_>, point: Point) -> Option<Target> {
        let display = env.device().display;
        let metrics = env.metrics();
        // How wide a share of the screen's width each band is; 0, the
        // gestures off.
        let share = env
            .settings()
            .int(crate::settings::LIGHT_SWIPE.as_str())
            .clamp(0, 50);
        if share == 0 {
            return None;
        }
        // What the kernel keeps for itself at the edges: the sides'
        // strip (page turns, back), and the bands above and below (the
        // quick panel comes down from the top, home up from the bottom).
        let mm = |setting: &Setting<i64>| -> i32 {
            let tenths = u32::try_from(env.settings().read(setting).clamp(0, 50)).unwrap_or(0);
            metrics.mm(tenths) as i32
        };
        let (across, sides) = (
            mm(&core_settings::EDGE_ENDS),
            mm(&core_settings::EDGE_SIDES),
        );
        if point.y < across || point.y >= i32::from(display.height) - across {
            return None;
        }
        let wide = i32::from(display.width) * share as i32 / 100;
        if wide <= sides {
            // Beside the kernel's strips nothing is left to take.
            return None;
        }
        let right = point.x >= i32::from(display.width) - wide;
        let ours = if right {
            point.x < i32::from(display.width) - sides
        } else {
            point.x < wide && point.x >= sides
        };
        if !ours {
            // On a strip the kernel keeps, or off the bands: not ours.
            return None;
        }
        // The right band is the brightness, the left one the warmth
        // (where there is one to set).
        if right {
            Target::of(env, 0)
        } else {
            Target::of(env, 1)
        }
    }

    /// The bands stand down: the finger taken let go, the tap held
    /// back dropped, and what its light showed without being kept is
    /// taken back where a context carries that answer.
    pub(super) fn let_go(&mut self, cx: &mut Cx<'_, '_>) {
        self.drag = None;
        self.tap = None;
        self.pending = None;
        if self.shown {
            self.shown = false;
            cx.command(Command::PreviewSettings(Vec::new()));
        }
    }

    /// The bands stand down for a time (another screen covers the
    /// reader): the finger taken let go, the tap held back dropped.
    /// Answers the values shown without being kept, for the context
    /// that asks what is left of them (`Screen::on_show` does).
    pub(super) fn covered(&mut self) -> bool {
        self.drag = None;
        self.tap = None;
        self.shown
    }
}

/// How long a held finger still has to make a long press. The kernel's
/// thresholds read the setting as it is; the reader panel's editing
/// keeps it between 100 and 5,000 ms, as a lift classification did.
fn long_press_ms(env: &Env<'_>) -> u64 {
    env.settings()
        .read(&core_settings::LONG_PRESS_MS)
        .clamp(100, 5_000) as u64
}

/// How long a tap waits for its second, as the kernel's holds one.
fn double_ms(env: &Env<'_>) -> u64 {
    env.settings()
        .read(&core_settings::DOUBLE_TAP_MS)
        .clamp(50, 2_000) as u64
}

/// Whether the key map's gestures answer on this screen now: the
/// screen's own gestures and the buttons always do; the kernel keeps
/// the key map's to itself under its own-gestures and gestures-off
/// policies (see `gestures::Manager::recognized`), and so do the bands.
fn gestures_standard(cx: &Cx<'_, '_>) -> bool {
    let off = ["core.gestures.own", "core.gestures.off"];
    bound(cx, "gestures.policy").is_none_or(|policy| !off.contains(&policy.as_str()))
}

/// The key map layers that answer for this screen, the most specific
/// first (as the kernel reads them): the screen's own, its slot's, the
/// global one.
fn layers<'c>(cx: &'c Cx<'_, '_>) -> Vec<&'c str> {
    let named = [cx.screen_id().map(Id::as_str), cx.slot().map(Id::as_str)];
    named
        .into_iter()
        .flatten()
        .chain(core::iter::once(GLOBAL_KEYMAP_LAYER))
        .collect()
}

/// What `input` is bound to on this screen.
fn bound(cx: &Cx<'_, '_>, input: &str) -> Option<Id> {
    let named = layers(cx);
    cx.env().keys().bound(&named, input)
}

impl ReaderScreen {
    /// A finger's sample of the book screen: taken in a band, and then
    /// told here until it is lifted.
    pub(super) fn on_light_pointer(
        &mut self,
        event: &PointerEvent,
        cx: &mut Cx<'_, '_>,
    ) -> Handled {
        let env = *cx.env();
        if !env.device().input.touch {
            return Handled::No;
        }
        match event.phase {
            PointerPhase::Down => self.took_of(&env, event, cx),
            PointerPhase::Move => self.moved_of(&env, event, cx),
            PointerPhase::Up | PointerPhase::Cancel => self.lifted(&env, event, cx),
        }
    }

    /// A finger coming down in a band: taken for the setting it names,
    /// where the reader has none taken and none elsewhere going.
    fn took_of(&mut self, env: &Env<'_>, event: &PointerEvent, cx: &mut Cx<'_, '_>) -> Handled {
        // A second finger is the pinch's; one on text being
        // chosen or a link being taken stays theirs; a touch
        // while reading aloud pauses it, as any tap does.
        if self.light_swipe.drag.is_some()
            || self.selection.is_some()
            || self.linking.is_some()
            || self.aloud.is_some()
        {
            return Handled::No;
        }
        let Some(target) = self.light_swipe.takes(env, event.point) else {
            return Handled::No;
        };
        let from = target.aligned(target.value(env));
        // A finger held still: the long press it may make is due when
        // the kernel's would (no second keeps it waiting).
        let hold = env.now().add_millis(long_press_ms(env));
        self.light_swipe.drag = Some(Drag {
            pointer: event.id,
            left: event.point.x < i32::from(env.device().display.width) / 2,
            target,
            start: event.point,
            at: env.now(),
            from,
            value: from,
            tool: event.tool,
            intent: None,
            shown: false,
            hold: Some(hold),
            fired: false,
        });
        self.input_at = Some(env.now());
        cx.wake_at(hold);
        Handled::Yes
    }

    /// A finger's move in a band: the slide says which value of the
    /// light is shown; a move along it is the light's, across the
    /// page's. The first to decide, the rest to follow.
    fn moved_of(&mut self, env: &Env<'_>, event: &PointerEvent, cx: &mut Cx<'_, '_>) -> Handled {
        let Some(drag) = self
            .light_swipe
            .drag
            .as_mut()
            .filter(|drag| drag.pointer == event.id)
        else {
            return Handled::No;
        };
        // How far a finger may wander and still tap: as much dead zone
        // as a tap may wander, so a touch of a band (a thumb picked up
        // to turn pages) flicks nothing.
        let slop = env.metrics().mm(u32::try_from(
            env.settings().read(&core_settings::TAP_SLOP).clamp(0, 50),
        )
        .unwrap_or(3)) as i32;
        let (dx, dy) = (event.point.x - drag.start.x, event.point.y - drag.start.y);
        if drag.intent.is_none() && dx.abs().max(dy.abs()) < slop.max(1) {
            return Handled::Yes;
        }
        // The long press ran at its time: its lift makes nothing (as
        // the kernel's), a move least, while the selection is shown.
        if drag.fired {
            return Handled::Yes;
        }
        if drag.intent.is_none() {
            // A slide along the band is the light's work; a move
            // across, at least as far, is the page's. Either way the
            // finger is on its way: no long press comes.
            drag.hold = None;
            drag.intent = Some(if dy.abs() > dx.abs() {
                Intent::Light
            } else {
                Intent::Turn
            });
        }
        if drag.intent == Some(Intent::Light) {
            let value = drag
                .target
                .shown(env, drag.from, drag.start.y - event.point.y);
            if value != drag.value {
                drag.value = value;
                drag.shown = true;
                self.light_swipe.shown = true;
                cx.command(Command::PreviewSettings(alloc::vec![(
                    drag.target.id.clone(),
                    Some(SettingValue::Int(value)),
                )]));
            }
        }
        Handled::Yes
    }

    /// A finger's end in a band: its lift or its taking-away.
    fn lifted(&mut self, env: &Env<'_>, event: &PointerEvent, cx: &mut Cx<'_, '_>) -> Handled {
        let Some(drag) = self
            .light_swipe
            .drag
            .take()
            .filter(|drag| drag.pointer == event.id)
        else {
            return Handled::No;
        };
        if event.phase == PointerPhase::Cancel {
            // The finger was taken away (the system's): the values
            // shown go, nothing is kept, nothing runs.
            if drag.shown {
                self.light_swipe.shown = false;
                cx.command(Command::PreviewSettings(Vec::new()));
            }
            return Handled::Yes;
        }
        if drag.fired {
            // The long press ran while the finger held: it does nothing
            // more, its lift least, as the kernel's.
            return Handled::Yes;
        }
        self.lift_of(*env, &drag, event, cx)
    }

    /// The finger comes to rest: the value kept where the band set one,
    /// the values shown without being kept let go, and everything else
    /// as it would have gone without the bands.
    fn lift_of(
        &mut self,
        env: Env<'_>,
        drag: &Drag,
        event: &PointerEvent,
        cx: &mut Cx<'_, '_>,
    ) -> Handled {
        if drag.shown {
            self.light_swipe.shown = false;
            cx.command(Command::PreviewSettings(Vec::new()));
        }
        match drag.intent {
            Some(Intent::Light) => {
                // The finger is judged where it was lifted, as a
                // slider's knob is.
                let value = drag
                    .target
                    .shown(&env, drag.from, drag.start.y - event.point.y);
                if value != drag.from {
                    cx.command(Command::SetSetting(
                        drag.target.id.clone(),
                        SettingValue::Int(value),
                    ));
                }
            }
            None | Some(Intent::Turn) => {
                self.lifted_of(env, drag, event, cx);
            }
        }
        Handled::Yes
    }

    /// Where the finger ends without the light having been set: what a
    /// tap, a swipe or a swipe in from its side would do there, if it
    /// would do anything at all.
    fn lifted_of(
        &mut self,
        env: Env<'_>,
        drag: &Drag,
        event: &PointerEvent,
        cx: &mut Cx<'_, '_>,
    ) -> Handled {
        let mm = |setting: &Setting<i64>| -> u32 {
            u32::try_from(env.settings().read(setting).clamp(0, 50)).unwrap_or(0)
        };
        let px = |value: u32| -> i32 { env.metrics().mm(value) as i32 };
        let (slop, swipe_distance) = (
            px(mm(&core_settings::TAP_SLOP)).max(1),
            px(mm(&core_settings::SWIPE_DISTANCE)).max(1),
        );
        let (dx, dy) = (event.point.x - drag.start.x, event.point.y - drag.start.y);
        if drag.intent == Some(Intent::Turn) {
            // In from the side the finger came down at, and more so than
            // across: as a swipe in from that side would be. Where it
            // does not qualify (outward, or askew), a stroke far enough
            // is a plain swipe, as the band would otherwise be; a
            // shorter one is nothing (an out-and-back drag is no tap).
            let inward = if drag.left { dx } else { -dx };
            if inward >= swipe_distance && dy.abs() < inward {
                self.edge_swipe(&env, drag.left, cx);
                return Handled::Yes;
            }
        }
        if dx * dx + dy * dy >= swipe_distance * swipe_distance {
            // A plain swipe, as the band would otherwise be: the
            // screen's own gestures answer first, then the key map's
            // where its gestures are answered on this screen.
            let direction = swipe_direction(drag.start, event.point);
            let gesture = Gesture::new(
                GestureKind::Swipe(direction),
                drag.start,
                event.point,
                drag.tool,
            );
            if self.on_gesture(gesture, cx) == Handled::Yes {
                return Handled::Yes;
            }
            if gestures_standard(cx) {
                let side = self.mirrored_side(direction);
                if let Some(action) = bound(cx, &alloc::format!("swipe.{}", side.id())) {
                    cx.command(Command::run(action));
                }
            }
            return Handled::Yes;
        }
        if drag.intent == Some(Intent::Turn) || dx * dx + dy * dy > slop * slop {
            // A turn's stroke that went nowhere, or between a tap and a
            // swipe: nothing, as it is elsewhere.
            return Handled::Yes;
        }
        // A tap where it stands: the screen's own answers first (a word
        // at the page's edge, a status line, the corner, the way back),
        // then what the zone it falls in has bound. A held one waited
        // for nothing: its long press ran at its time while the finger
        // held (see `light_tick`), so a lift this late cannot be one.
        let kind = if env.now().since(drag.at) >= long_press_ms(&env) {
            GestureKind::LongPress
        } else {
            GestureKind::Tap
        };
        let gesture = Gesture::new(kind, drag.start, event.point, drag.tool);
        if self.on_gesture(gesture, cx) == Handled::Yes {
            return Handled::Yes;
        }
        self.zone_tap(&env, drag.start, kind == GestureKind::LongPress, cx);
        Handled::Yes
    }

    /// What a tap (or long tap) of a band runs on the key map, keeping
    /// what the kernel keeps of its dispatch: nothing of it under its
    /// own-gestures and gestures-off policies; the zone of the ordinary
    /// grid, mirrored where the pages go the other way round; nothing
    /// where it is bound to nothing.
    fn zone_tap(&mut self, env: &Env<'_>, at: Point, long: bool, cx: &mut Cx<'_, '_>) {
        if !gestures_standard(cx) {
            return;
        }
        let display = env.device().display;
        let mut zone = zone_of(at, display.width, display.height);
        if self.mirrored() {
            zone = other_zone(zone);
        }
        if long {
            // No double of a long tap is looked for, as the kernel does
            // not.
            if let Some(action) = bound(cx, &alloc::format!("long-tap.{zone}")) {
                cx.command(Command::run(action));
            }
            return;
        }
        let single = alloc::format!("tap.{zone}");
        let double = alloc::format!("double-tap.{zone}");
        if bound(cx, &double).is_none() {
            // Nothing waits for a second: it runs now. A tap held back
            // for another zone's double runs first, as the kernel's own
            // dispatch keeps ([`gestures::Manager::route_key_pending`]).
            if let Some((held, _)) = self.light_swipe.tap.take()
                && let Some(action) = bound(cx, &held)
            {
                cx.command(Command::run(action));
            }
            if let Some(action) = bound(cx, &single) {
                cx.command(Command::run(action));
            }
            return;
        }
        // Something is bound to the second tap of the zone: the first is
        // held back for it ([`double_ms`]), the second makes it, none
        // makes the single when the time is out, and a first tap of
        // another zone runs when a new one holds it back — as the
        // kernel's holds one ([`gestures::Manager::single_or_double`]).
        let now = env.now();
        let until = now.add_millis(double_ms(env));
        match self.light_swipe.tap.take() {
            Some((held, holding)) if held == single && now < holding => {
                if let Some(action) = bound(cx, &double) {
                    cx.command(Command::run(action));
                }
            }
            other => {
                // The tap held back runs first, as it came first.
                if let Some((held, _)) = other
                    && let Some(action) = bound(cx, &held)
                {
                    cx.command(Command::run(action));
                }
                self.light_swipe.tap = Some((single, until));
                cx.wake_at(until);
            }
        }
    }

    /// What a swipe in from a band's side runs, as the kernel reads its
    /// own edges (the key map layers shown, the side the other way where
    /// the pages go the other way round); where the reader's gestures
    /// are the screen's own, a second one within a few seconds first.
    fn edge_swipe(&mut self, env: &Env<'_>, left: bool, cx: &mut Cx<'_, '_>) -> Handled {
        let side = if left {
            Direction::Left
        } else {
            Direction::Right
        };
        // Where the screen's gestures are off, the kernel's edges are
        // none (`gestures::Manager::edge_at` declines them before the
        // finger is followed), so a band's stroke is none either. Under
        // the screen's own, a swipe in from an edge needs its second
        // one within a few seconds, a band's the same; under the
        // standard one, it is what the edge is bound to.
        let bound_policy = bound(cx, "gestures.policy");
        let policy = bound_policy.as_ref().map(|id| id.as_str());
        if policy == Some("core.gestures.off") {
            return Handled::Yes;
        }
        if policy == Some("core.gestures.own") {
            // As the kernel's own edges (`gestures::Manager::edge_at`)
            // decline a side nothing is bound to, a band's stroke in
            // from it is none either: no second one waited for, no
            // hint of one.
            if bound(
                cx,
                &alloc::format!("edge.{}", self.mirrored_side(side).id()),
            )
            .is_none()
            {
                return Handled::Yes;
            }
            let again = self
                .light_swipe
                .pending
                .take()
                .is_some_and(|(pending, until)| pending == side && env.now() < until);
            if !again {
                self.light_swipe.pending = Some((side, env.now().add_millis(EDGE_AGAIN_MS)));
                cx.command(Command::Notify(
                    env.i18n().tr("core.edge-again").to_string(),
                ));
                return Handled::Yes;
            }
        }
        if let Some(action) = bound(
            cx,
            &alloc::format!("edge.{}", self.mirrored_side(side).id()),
        ) {
            cx.command(Command::run(action));
        }
        Handled::Yes
    }

    /// The side the other way where the pages go the other way round, as
    /// the kernel reads swipes ([`Screen::mirrored`]).
    fn mirrored_side(&self, side: Direction) -> Direction {
        if self.mirrored() {
            match side {
                Direction::Left => Direction::Right,
                Direction::Right => Direction::Left,
                other => other,
            }
        } else {
            side
        }
    }

    /// The time the light's sides ask for, at the reader's own time:
    /// the long press of a finger held still in a band (the kernel's
    /// recognizers never saw the finger, so their own time cannot fire
    /// it), and the single tap of a band held back for its second.
    pub(super) fn light_tick(&mut self, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let now = env.now();
        // The long press: held still and alone, its time come.
        let held = self.light_swipe.drag.as_ref().and_then(|drag| {
            drag.hold
                .map(|due| (due, drag.start, drag.tool, drag.intent))
        });
        if let Some((due, start, tool, intent)) = held {
            if now < due {
                cx.wake_at(due);
            } else if intent.is_none() {
                // The long press runs at its time, where its finger
                // never moved: the screen's own answers first (a word
                // turned into), then the zone's long tap.
                let drag = self.light_swipe.drag.as_mut();
                if let Some(drag) = drag {
                    drag.hold = None;
                    drag.fired = true;
                }
                let gesture = Gesture::new(GestureKind::LongPress, start, start, tool);
                if self.on_gesture(gesture, cx) != Handled::Yes {
                    self.zone_tap(&env, start, true, cx);
                }
            } else {
                // It moved while the time ran out: the long press is
                // off, the lift's work already decided and kept.
                if let Some(drag) = self.light_swipe.drag.as_mut() {
                    drag.hold = None;
                }
            }
        }
        // The single tap of a band held back for its second: none came
        // in time, so the first is what it was.
        let tap = self.light_swipe.tap.clone();
        if let Some((single, until)) = tap {
            if now < until {
                cx.wake_at(until);
            } else {
                self.light_swipe.tap = None;
                if let Some(action) = bound(cx, &single) {
                    cx.command(Command::run(action));
                }
            }
        }
    }
}
