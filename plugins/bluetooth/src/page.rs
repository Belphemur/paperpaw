//! The Bluetooth page (the settings group's own page): the devices
//! bonded with this one (a remote's or a keyboard's keys switched on and
//! off by choosing it; the menu button or a long press forgets one),
//! adding a device, when remotes connect, and the plugins each device is
//! given to.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use paperpaw_sdk::bluetooth::{self, Bonded, DeviceId, DeviceKind};
use paperpaw_sdk::builtin::{actions, events};
use paperpaw_sdk::ui::{self, ListView, UiStyle};
use paperpaw_sdk::{
    Arg, Canvas, Command, Cx, Env, Gesture, GestureKind, Handled, Hint, Id, Params, Rect,
    RefreshMode, Screen, Uses,
};

use crate::{DONE, REMOTES, REMOTES_CHOICES, SEARCH};

/// A row of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    Device(DeviceId),
    /// Gives the device above to the plugin, or takes it back.
    Give(DeviceId, Id),
    Add,
    Remotes,
}

pub(crate) struct BluetoothPage {
    style: UiStyle,
    header: ui::Header,
    list: ListView,
}

/// The device's kind, in words.
pub(crate) fn kind_text<'e>(env: &Env<'e>, kind: DeviceKind) -> &'e str {
    env.i18n().tr(match kind {
        DeviceKind::Keyboard => "bluetooth.kind.keyboard",
        DeviceKind::Remote => "bluetooth.kind.remote",
        DeviceKind::Sensor => "bluetooth.kind.sensor",
        DeviceKind::Phone => "bluetooth.kind.phone",
        _ => "bluetooth.kind.other",
    })
}

/// What is known of a bonded device, in words: its kind, connected.
fn device_note(env: &Env<'_>, device: &Bonded) -> String {
    let mut parts = alloc::vec![kind_text(env, device.kind)];
    if device.connected {
        parts.push(env.i18n().tr("bluetooth.connected"));
    }
    parts.join(" · ")
}

/// The plugins besides this one that use Bluetooth: a device may be
/// given to them.
fn users(env: &Env<'_>) -> Vec<(Id, String)> {
    env.plugins()
        .iter()
        .filter(|p| p.manifest.uses.contains(Uses::BLUETOOTH) && p.manifest.id != crate::PLUGIN)
        .map(|p| (p.manifest.id.clone(), p.name(env.i18n()).to_string()))
        .collect()
}

impl BluetoothPage {
    pub(crate) fn new(env: &Env<'_>) -> Self {
        let style = UiStyle::new(env);
        let display = env.device().display;
        let header = ui::Header::new(env, &style, true);
        let top = header.area().height + style.gap();
        let area = Rect::new(0, top, display.width, display.height.saturating_sub(top));
        // Rows as high as the add row's and the remotes' need, their notes
        // whole (the largest text).
        let tr = |key| env.i18n().tr(key);
        let widest = REMOTES_CHOICES
            .iter()
            .map(|c| tr(c))
            .max_by_key(|c| c.len())
            .unwrap_or_default();
        let add = ui::Row::value(tr("bluetooth.add"), "›").note(tr("bluetooth.add.help"));
        let remotes =
            ui::Row::value(tr("bluetooth.remotes"), widest).note(tr("bluetooth.remotes.help"));
        let row =
            add.height(env, &style, display.width)
                .max(remotes.height(env, &style, display.width));
        Self {
            list: ListView::new(area, u32::from(row).max(style.noted_row())).focus_on_keys(env),
            style,
            header,
        }
    }

    fn rows(env: &Env<'_>) -> Vec<Row> {
        let mut rows = Vec::new();
        let users = users(env);
        let view = env.bluetooth();
        for device in view.iter().flat_map(|b| b.bonded()) {
            rows.push(Row::Device(device.id));
            // Sensors and the like are for plugins: given to one or more.
            if !device.kind.can_type() {
                for (plugin, _) in &users {
                    rows.push(Row::Give(device.id, plugin.clone()));
                }
            }
        }
        rows.push(Row::Add);
        rows.push(Row::Remotes);
        rows
    }

    fn changed(cx: &mut Cx<'_, '_>) {
        cx.request_redraw();
        cx.request_refresh(RefreshMode::Fast);
    }

    /// What choosing a row does.
    fn activate(&mut self, index: usize, cx: &mut Cx<'_, '_>) {
        let env = *cx.env();
        let Some(row) = Self::rows(&env).into_iter().nth(index) else {
            return;
        };
        let bond = |id| env.bluetooth().and_then(|b| b.bond(id));
        match row {
            Row::Device(id) => {
                if let Some(device) = bond(id)
                    && device.kind.can_type()
                {
                    cx.command(if device.keys {
                        bluetooth::remote_keys_off(id, &DONE)
                    } else {
                        bluetooth::remote_keys(id, &DONE)
                    });
                }
            }
            Row::Give(id, plugin) => {
                let given = env
                    .bluetooth()
                    .is_some_and(|b| b.given(id).contains(&plugin));
                cx.command(if given {
                    bluetooth::take_back(id, plugin, &DONE)
                } else {
                    bluetooth::give(id, plugin, &DONE)
                });
            }
            Row::Add => {
                cx.command(Command::OpenScreen(SEARCH, Params::new()));
                return;
            }
            Row::Remotes => {
                let now = env.settings().read(&REMOTES);
                let at = REMOTES_CHOICES.iter().position(|c| *c == now).unwrap_or(0);
                let next = REMOTES_CHOICES[(at + 1) % REMOTES_CHOICES.len()];
                cx.command(Command::set(&REMOTES, alloc::borrow::Cow::Borrowed(next)));
            }
        }
        Self::changed(cx);
    }

    /// Forgets the device in row `index`.
    fn forget(&self, index: usize, cx: &mut Cx<'_, '_>) -> bool {
        let env = *cx.env();
        let Some(Row::Device(id)) = Self::rows(&env).into_iter().nth(index) else {
            return false;
        };
        let name = env
            .bluetooth()
            .and_then(|b| b.bond(id))
            .map(|d| d.name.clone())
            .unwrap_or_default();
        cx.command(bluetooth::forget(id, &DONE));
        cx.command(Command::Notify(
            env.i18n()
                .format("bluetooth.forgotten", &[("name", Arg::Str(&name))]),
        ));
        Self::changed(cx);
        true
    }
}

impl Screen for BluetoothPage {
    fn draw(&mut self, canvas: &mut Canvas<'_>, env: &Env<'_>) {
        let style = self.style;
        let rows = Self::rows(env);
        let len = rows.len();
        let title = env.i18n().tr("bluetooth.title");
        let mut ui = ui::Ui::new(canvas, env, &style);
        self.header
            .draw(&mut ui, title, None, self.list.paging(len));
        let users = users(env);
        self.list.draw(&mut ui, len, |ui, row, rect, focused| {
            let tr = |key| env.i18n().tr(key);
            let bond = |id| env.bluetooth().and_then(|b| b.bond(id));
            match &rows[row] {
                Row::Device(id) => {
                    let Some(device) = bond(*id) else { return };
                    let note = device_note(env, device);
                    if device.kind.can_type() {
                        ui::Row::switch(&device.name, device.keys)
                            .note(&note)
                            .focused(focused)
                            .draw(ui, rect);
                    } else {
                        ui::Row::plain(&device.name)
                            .note(&note)
                            .focused(focused)
                            .draw(ui, rect);
                    }
                }
                Row::Give(id, plugin) => {
                    let name = users
                        .iter()
                        .find(|(p, _)| p == plugin)
                        .map_or("", |(_, n)| n.as_str());
                    let title = env
                        .i18n()
                        .format("bluetooth.give", &[("plugin", Arg::Str(name))]);
                    let given = env
                        .bluetooth()
                        .is_some_and(|b| b.given(*id).contains(plugin));
                    ui::Row::switch(&title, given)
                        .focused(focused)
                        .draw(ui, rect);
                }
                Row::Add => ui::Row::value(tr("bluetooth.add"), "›")
                    .note(tr("bluetooth.add.help"))
                    .focused(focused)
                    .draw(ui, rect),
                Row::Remotes => {
                    ui::Row::value(tr("bluetooth.remotes"), tr(env.settings().read(&REMOTES)))
                        .note(tr("bluetooth.remotes.help"))
                        .focused(focused)
                        .draw(ui, rect)
                }
            }
        });
    }

    fn on_event(&mut self, event: &Id, _: &Params, cx: &mut Cx<'_, '_>) {
        if *event == events::BLUETOOTH_CHANGED || *event == events::SETTING_CHANGED {
            Self::changed(cx);
        }
    }

    fn hint(&self, action: &Id, env: &Env<'_>) -> Hint {
        let row = Self::rows(env).into_iter().nth(self.list.focus());
        match row {
            Some(Row::Device(_)) if *action == actions::MENU => {
                Hint::Text(env.i18n().tr("bluetooth.forget").to_string())
            }
            _ => Hint::Default,
        }
    }

    fn on_action(&mut self, action: &Id, cx: &mut Cx<'_, '_>) -> Handled {
        let len = Self::rows(cx.env()).len();
        let rows = self.list.rows() as isize;
        let a = action;
        let delta = if let Some(delta) = ui::list_step(a, rows) {
            delta
        } else if *a == actions::CONFIRM {
            self.activate(self.list.focus(), cx);
            return Handled::Yes;
        } else if *a == actions::MENU && self.forget(self.list.focus(), cx) {
            return Handled::Yes;
        } else {
            return Handled::No;
        };
        self.list.move_focus_in(cx.env(), delta, len).request(cx);
        Handled::Yes
    }

    fn on_gesture(&mut self, gesture: Gesture, cx: &mut Cx<'_, '_>) -> Handled {
        let len = Self::rows(cx.env()).len();
        let paging = self.list.paging(len);
        if gesture.kind == GestureKind::Tap
            && let Some(action) = self
                .header
                .tap(cx.env(), &self.style, gesture.start, paging)
        {
            cx.command(Command::run(action));
            return Handled::Yes;
        }
        let Some(index) = self.list.hit(gesture.start, len) else {
            return Handled::No;
        };
        self.list.set_focus(index);
        match gesture.kind {
            GestureKind::Tap => {
                self.activate(index, cx);
                Handled::Yes
            }
            // A long press forgets a device (the menu button's part on
            // devices without touch).
            GestureKind::LongPress if self.forget(index, cx) => Handled::Yes,
            _ => Handled::No,
        }
    }
}
