//! "Serial console…": pick a host (this PC or a remote one), a port and a
//! speed, and open the port in a terminal tab. Ports show who has them open:
//! two programs reading one port split its data and both see garbage.
//!
//! Local ports are not opened to find out (opening toggles DTR, which resets
//! many boards): Windows lets one program have a port at a time, so a busy
//! one fails to open and the tab says so. Remote ports: the agent lists the
//! processes that have them open, and the bridge asks before joining them.

use crate::app::{App, Pending};
use crate::host::{HostKind, Profile, BAUDS};
use crate::theme;
use egui::{vec2, RichText, ScrollArea, TextEdit};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq)]
pub struct PortInfo {
    pub port: String,
    pub desc: String,
    /// (pid, user, command) of other processes that have it open.
    pub holders: Vec<(u64, String, String)>,
}

pub struct SerialDialog {
    pub host: String,
    /// None: being asked for.
    pub ports: Option<Result<Vec<PortInfo>, String>>,
    pub port: String,
    pub baud: u32,
}

impl SerialDialog {
    pub fn new() -> SerialDialog {
        SerialDialog { host: "local".into(), ports: None, port: String::new(), baud: 115_200 }
    }
}

/// The ports of this PC (no opening, see above).
fn local_ports() -> Result<Vec<PortInfo>, String> {
    let mut v: Vec<PortInfo> = serialport::available_ports()
        .map_err(|e| e.to_string())?
        .into_iter()
        .map(|p| {
            let desc = match p.port_type {
                serialport::SerialPortType::UsbPort(u) => [u.manufacturer, u.product].into_iter().flatten().collect::<Vec<_>>().join(" "),
                serialport::SerialPortType::BluetoothPort => "Bluetooth".into(),
                serialport::SerialPortType::PciPort => "PCI".into(),
                serialport::SerialPortType::Unknown => String::new(),
            };
            PortInfo { port: p.port_name, desc, holders: Vec::new() }
        })
        .collect();
    v.sort_by(|a, b| a.port.len().cmp(&b.port.len()).then_with(|| a.port.cmp(&b.port)));
    Ok(v)
}

/// The agent's answer to "serial_ports".
pub fn parse_ports(r: &Value) -> Vec<PortInfo> {
    r["ports"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|p| PortInfo {
                    port: p["port"].as_str().unwrap_or("").into(),
                    desc: p["desc"].as_str().unwrap_or("").into(),
                    holders: p["holders"]
                        .as_array()
                        .map(|h| h.iter().map(|x| (x["pid"].as_u64().unwrap_or(0), x["user"].as_str().unwrap_or("").into(), x["cmd"].as_str().unwrap_or("").into())).collect())
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

impl App {
    pub fn open_serial_dialog(&mut self) {
        self.serial_dialog = Some(SerialDialog::new());
        self.ask_serial_ports();
    }

    fn ask_serial_ports(&mut self) {
        let Some(d) = self.serial_dialog.as_mut() else { return };
        if d.host == "local" {
            d.ports = Some(local_ports());
        } else {
            d.ports = None;
            let host = d.host.clone();
            self.request(&host, json!({"op": "serial_ports"}), Pending::SerialPorts { host: host.clone() });
        }
    }

    /// The tab for a port: a host profile kept in settings, so the tab comes
    /// back with the session (and a restart) like any other.
    pub fn open_serial(&mut self, via: &str, port: &str, baud: u32) {
        let name = if via == "local" { port.to_string() } else { format!("{via}:{port}") };
        let kind = HostKind::Serial { via: via.into(), port: port.into(), baud };
        match self.settings.profiles.iter_mut().find(|p| p.name == name) {
            Some(p) => p.kind = kind,
            None => self.settings.profiles.push(Profile { name: name.clone(), kind }),
        }
        self.save_settings();
        self.new_tab(&name, None);
    }

    /// Tabs of this weft that have `port` on `via` open.
    fn serial_tab(&self, via: &str, port: &str) -> Option<String> {
        self.panes.values().find_map(|p| match self.profile(&p.host).kind {
            HostKind::Serial { via: v, port: q, .. } if v == via && q == port && !p.exited => Some(p.title()),
            _ => None,
        })
    }

    pub fn ui_serial_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut d) = self.serial_dialog.take() else { return };
        let mut open = true;
        let mut ask = false;
        let mut go: Option<(String, String, u32)> = None;
        let hosts: Vec<String> = std::iter::once("local".to_string())
            .chain(self.settings.profiles.iter().filter(|p| matches!(p.kind, HostKind::Ssh { .. } | HostKind::Custom { .. })).map(|p| p.name.clone()))
            .collect();
        crate::widgets::dialog(ctx, "Serial console", vec2(520.0, 460.0)).open(&mut open).resizable(true).show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Host");
                let before = d.host.clone();
                egui::ComboBox::from_id_salt("serial-host").selected_text(&d.host).show_ui(ui, |ui| {
                    for h in &hosts {
                        ui.selectable_value(&mut d.host, h.clone(), if h == "local" { "local (this PC)" } else { h.as_str() });
                    }
                });
                if d.host != before {
                    ask = true;
                    d.port.clear();
                }
                if crate::icons::button(ui, crate::icons::Icon::Refresh, "list the ports again").clicked() {
                    ask = true;
                }
            });
            ui.add_space(4.0);
            ScrollArea::vertical().max_height(240.0).auto_shrink([false, true]).show(ui, |ui| match &d.ports {
                None => {
                    ui.label(RichText::new(format!("asking {}…", d.host)).color(theme::text_faint()));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(theme::error()));
                }
                Some(Ok(ports)) if ports.is_empty() => {
                    ui.label(RichText::new("no serial ports found — type one below").color(theme::text_faint()));
                }
                Some(Ok(ports)) => {
                    for p in ports {
                        let ours = self.serial_tab(&d.host, &p.port);
                        let busy = !p.holders.is_empty() || ours.is_some();
                        let sel = d.port == p.port;
                        let r = ui.horizontal(|ui| {
                            let r = ui.selectable_label(sel, RichText::new(&p.port).monospace().color(if busy { theme::warn() } else { theme::text() }));
                            if !p.desc.is_empty() {
                                ui.label(RichText::new(&p.desc).size(11.5).color(theme::text_dim()));
                            }
                            r
                        });
                        let r = r.inner;
                        if let Some(t) = &ours {
                            ui.label(RichText::new(format!("   open in this weft: \"{t}\"")).size(11.5).color(theme::warn()));
                        }
                        for (pid, user, cmd) in &p.holders {
                            ui.label(RichText::new(format!("   in use: pid {pid} · {user} · {cmd}")).size(11.5).color(theme::warn()));
                        }
                        if r.clicked() {
                            d.port = p.port.clone();
                        }
                        if r.double_clicked() {
                            go = Some((d.host.clone(), p.port.clone(), d.baud));
                        }
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.label("Port");
                // The first port listed as the example (each system names its
                // own: COM3, /dev/ttyUSB0, /dev/cu.usbserial-…).
                let hint = d.ports.as_ref().and_then(|r| r.as_ref().ok()).and_then(|v| v.first()).map(|p| p.port.clone()).unwrap_or_default();
                ui.add(TextEdit::singleline(&mut d.port).hint_text(hint).desired_width(170.0));
                ui.label("Speed");
                egui::ComboBox::from_id_salt("serial-baud").selected_text(d.baud.to_string()).show_ui(ui, |ui| {
                    for b in BAUDS {
                        ui.selectable_value(&mut d.baud, *b, b.to_string());
                    }
                });
                if ui.add_enabled(!d.port.trim().is_empty(), egui::Button::new("Open")).on_hover_text("in a new tab; Ctrl+] ends a remote console").clicked() {
                    go = Some((d.host.clone(), d.port.trim().to_string(), d.baud));
                }
            });
            ui.label(
                RichText::new("8 data bits, no parity, 1 stop bit. A port in use: Windows refuses it; a remote one asks first (two readers garble the data).")
                    .size(11.0)
                    .color(theme::text_faint()),
            );
        });
        self.serial_dialog = Some(d);
        if ask {
            self.ask_serial_ports();
        }
        if let Some((via, port, baud)) = go {
            self.serial_dialog = None;
            self.open_serial(&via, &port, baud);
        } else if !open {
            self.serial_dialog = None;
        }
    }
}
