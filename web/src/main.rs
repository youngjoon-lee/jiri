use std::{collections::VecDeque, time::Duration};

use egui::{Color32, RichText};
use futures::{channel::mpsc, SinkExt};
use jiri_core::p2p::{self, command, event, message};
use multiaddr::Multiaddr;
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

fn main() {
    // Make sure panics are logged using `console.error`.
    console_error_panic_hook::set_once();
    console_log!("Starting jiri-web...");

    let web_options = eframe::WebOptions::default();
    spawn_local(async {
        eframe::start_web(
            "jiri_canvas",
            web_options,
            Box::new(|cc| Box::new(MainApp::new(cc))),
        )
        .await
        .expect("failed to start eframe");
    });
}

pub struct MainApp {
    event_rx: async_channel::Receiver<event::Event>,
    command_tx: mpsc::UnboundedSender<command::Command>,
    messages: VecDeque<(Color32, String)>,
    connected: bool,
    text: String,
}

impl MainApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let (core, command_tx, event_rx) = p2p::Core::new().unwrap();
        let run_core = || async move { core.run().await.unwrap() };
        spawn_local(run_core());

        Self {
            event_rx,
            command_tx,
            messages: Default::default(),
            connected: false,
            text: "/ip4/127.0.0.1/tcp/8081/ws".to_string(),
        }
    }

    fn send_command(&self, command: command::Command) {
        let mut tx = self.command_tx.clone();
        spawn_local(async move {
            let _ = tx.send(command).await;
        });
    }

    fn send_dial(&mut self) {
        if let Ok(address) = self.text.parse::<Multiaddr>() {
            self.send_command(command::Command::Dial(address));
        } else {
            self.messages.push_back((
                Color32::RED,
                format!("Invalid multiaddr {}", self.text.clone()),
            ));
        }
    }

    fn send_chat(&mut self) {
        self.send_command(command::Command::SendMessage(message::Message::Text(
            self.text.clone(),
        )));
        self.messages
            .push_back((Color32::LIGHT_BLUE, format!("{: >20}", self.text)));
        self.text.clear();
    }
}

impl eframe::App for MainApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                event::Event::Msg(message) => match message {
                    message::Message::Text(text) => self.messages.push_back((Color32::GREEN, text)),
                    _ => {} //TODO
                },
                event::Event::Connected(peer_id) => {
                    self.connected = true;
                    self.text.clear();
                    self.messages
                        .push_back((Color32::YELLOW, format!("Connected to {peer_id}")));
                }
                event::Event::Disconnected(peer_id) => {
                    self.connected = true;
                    self.messages
                        .push_back((Color32::YELLOW, format!("Disconnected to {peer_id}")));
                }
                event::Event::Error(e) => {
                    self.connected = false;
                    self.messages
                        .push_back((Color32::RED, format!("Error: {e}")));
                }
            }
        }

        // Render command panel
        egui::TopBottomPanel::bottom("command").show(ctx, |ui| {
            ui.horizontal(|ui| {
                let resp = ui.text_edit_singleline(&mut self.text);
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if self.connected {
                        self.send_chat();
                    } else {
                        self.send_dial();
                    }
                }

                if self.connected {
                    if ui.button("Chat").clicked() {
                        self.send_chat();
                    }
                } else {
                    if ui.button("Connect").clicked() {
                        self.send_dial();
                    }
                }
            });
        });

        // Render message panel
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false; 2])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for (color, text) in &self.messages {
                        ui.label(RichText::new(text).size(14.0).monospace().color(*color));
                    }
                    ui.allocate_space(ui.available_size());
                });
        });

        // Run 20 frames/sec
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}
