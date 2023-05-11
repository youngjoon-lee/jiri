use std::{collections::VecDeque, time::Duration};

use egui::{Color32, RichText};
use futures::{channel::mpsc, SinkExt};
use jiri_wasm::{self, command, event};
use wasm_bindgen::prelude::wasm_bindgen;
use wasm_bindgen_futures::spawn_local;

// Debugging console log.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

macro_rules! console_log {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

/// We derive Deserialize/Serialize so we can persist app state on shutdown.
// #[derive(serde::Deserialize, serde::Serialize)]
// #[serde(default)] // if we add new fields, give them default values when deserializing old state
pub struct JiriWebApp {
    // Example stuff:
    label: String,

    // this how you opt-out of serialization of a member
    // #[serde(skip)]
    value: f32,

    remote_multiaddr: String,
    connected: bool,
    command_tx: Option<mpsc::UnboundedSender<command::Command>>,
    event_rx: Option<mpsc::UnboundedReceiver<event::Event>>,

    message: String,
    messages: VecDeque<(Color32, String)>,
}

impl Default for JiriWebApp {
    fn default() -> Self {
        Self {
            // Example stuff:
            label: "Hello World!".to_owned(),
            value: 2.7,
            remote_multiaddr: "".to_owned(),
            connected: false,
            command_tx: None,
            event_rx: None,

            message: "".to_owned(),
            messages: Default::default(),
        }
    }
}

impl JiriWebApp {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // This is also where you can customize the look and feel of egui using
        // `cc.egui_ctx.set_visuals` and `cc.egui_ctx.set_fonts`.

        // Load previous app state (if any).
        // Note that you must enable the `persistence` feature for this to work.
        // if let Some(storage) = cc.storage {
        //     return eframe::get_value(storage, eframe::APP_KEY).unwrap_or_default();
        // }

        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        Default::default()
    }

    fn send_command(&self, cmd: command::Command) {
        if let Some(command_tx) = &self.command_tx {
            let mut tx = command_tx.clone();
            spawn_local(async move {
                let _ = tx.send(cmd).await;
            });
        }
    }

    fn send_message(&mut self) {
        self.send_command(command::Command::SendMessage(self.message.clone()));
        self.messages
            .push_back((Color32::LIGHT_BLUE, self.message.clone()));
    }
}

impl eframe::App for JiriWebApp {
    /// Called by the frame work to save state before shutdown.
    // fn save(&mut self, storage: &mut dyn eframe::Storage) {
    //     eframe::set_value(storage, eframe::APP_KEY, self);
    // }

    /// Called each time the UI needs repainting, which may be many times per second.
    /// Put your widgets into a `SidePanel`, `TopPanel`, `CentralPanel`, `Window` or `Area`.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Examples of how to create different panels and windows.
        // Pick whichever suits you.
        // Tip: a good default choice is to just keep the `CentralPanel`.
        // For inspiration and more examples, go to https://emilk.github.io/egui

        if let Some(event_rx) = &mut self.event_rx {
            while let Ok(Some(event)) = event_rx.try_next() {
                match event {
                    event::Event::Message(msg) => {
                        console_log!("EVENT: MESSAGE: {}", msg.clone());
                        self.messages.push_back((Color32::GREEN, msg.clone()));
                    }
                    event::Event::Connected(multiaddr) => {
                        console_log!("EVENT: Connected to {multiaddr}");
                        self.connected = true;
                    }
                }
            }
        }

        // #[cfg(not(target_arch = "wasm32"))] // no File->Quit on web pages!
        egui::TopBottomPanel::top("top_panel").show(ctx, |ui| {
            // The top panel is often a good place for a menu bar:
            // egui::menu::bar(ui, |ui| {
            //     ui.menu_button("File", |ui| {
            //         if ui.button("Quit").clicked() {
            //             _frame.close();
            //         }
            //     });
            // });

            ui.heading("JIRI WASM Web");

            ui.horizontal(|ui| {
                ui.label("A multiaddr to connect to: ");
                ui.text_edit_singleline(&mut self.remote_multiaddr);
                if !self.connected {
                    if ui.button("Connect").clicked() {
                        let (command_tx, event_rx) =
                            jiri_wasm::start_interactive(self.remote_multiaddr.clone());
                        self.command_tx = Some(command_tx);
                        self.event_rx = Some(event_rx);
                    }
                } else {
                    ui.label("Connected");
                }
            });
        });

        // egui::SidePanel::left("side_panel").show(ctx, |ui| {
        //     ui.heading("Side Panel");
        //
        //     ui.horizontal(|ui| {
        //         ui.label("Write something: ");
        //         ui.text_edit_singleline(&mut self.label);
        //     });
        //
        //     ui.add(egui::Slider::new(&mut self.value, 0.0..=10.0).text("value"));
        //     if ui.button("Increment").clicked() {
        //         self.value += 1.0;
        //     }
        //
        //     ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
        //         ui.horizontal(|ui| {
        //             ui.spacing_mut().item_spacing.x = 0.0;
        //             ui.label("powered by ");
        //             ui.hyperlink_to("egui", "https://github.com/emilk/egui");
        //             ui.label(" and ");
        //             ui.hyperlink_to(
        //                 "eframe",
        //                 "https://github.com/emilk/egui/tree/master/crates/eframe",
        //             );
        //             ui.label(".");
        //         });
        //     });
        // });

        egui::CentralPanel::default().show(ctx, |ui| {
            // The central panel the region left after adding TopPanel's and SidePanel's

            // ui.heading("eframe template");
            // ui.hyperlink("https://github.com/emilk/eframe_template");
            // ui.add(egui::github_link_file!(
            //     "https://github.com/emilk/eframe_template/blob/master/",
            //     "Source code."
            // ));
            // egui::warn_if_debug_build(ui);

            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (color, msg) in &self.messages {
                        ui.label(RichText::new(msg).color(*color));
                    }
                    ui.allocate_space(ui.available_size());
                });
        });

        egui::TopBottomPanel::bottom("bottom_panel").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let resp = ui.text_edit_singleline(&mut self.message);
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.send_message();
                }

                if ui.button("Send").clicked() {
                    self.send_message();
                }
            });
        });

        if false {
            egui::Window::new("Window").show(ctx, |ui| {
                ui.label("Windows can be moved by dragging them.");
                ui.label("They are automatically sized based on contents.");
                ui.label("You can turn on resizing and scrolling if you like.");
                ui.label("You would normally choose either panels OR windows.");
            });
        }

        // Run 20 frames/sec
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}
