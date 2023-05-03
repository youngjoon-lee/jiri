use std::{collections::VecDeque, time::Duration};

use egui::{Color32, RichText};
use futures::{
    channel::{mpsc, oneshot},
    SinkExt,
};
use jiri_core::p2p::{self, command, event, message};
use multiaddr::Multiaddr;
use uuid::Uuid;
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
    peer_id: String,
    event_rx: async_channel::Receiver<event::Event>,
    command_tx: mpsc::UnboundedSender<command::Command>,
    messages: VecDeque<(Color32, String)>,
    connected: bool,
    text: String,
    file: String,
}

impl MainApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let (core, command_tx, event_rx) = p2p::Core::new().unwrap();
        let peer_id = core.peer_id.to_string();
        let run_core = || async move { core.run().await.unwrap() };
        spawn_local(run_core());

        Self {
            peer_id,
            event_rx,
            command_tx,
            messages: Default::default(),
            connected: false,
            text: "/ip4/127.0.0.1/tcp/8081/ws".to_string(),
            file: "put file contents".to_string(),
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
        self.send_command(command::Command::SendMessage(message::Message::Text {
            source_peer_id: self.peer_id.clone(),
            text: self.text.clone(),
        }));
        self.messages.push_back((
            Color32::LIGHT_BLUE,
            format!("{}: {}", self.peer_id, self.text),
        ));
        self.text.clear();
    }

    fn send_file(&mut self) {
        let file_name = Uuid::new_v4().to_string();
        let file = self.text.clone().into_bytes();
        let (oneshot_tx, oneshot_rx) = oneshot::channel();
        let mut command_tx = self.command_tx.clone();

        spawn_local(async move {
            let _ = command_tx
                .send(command::Command::StartFileProviding {
                    file_name: file_name.clone(),
                    file,
                    sender: oneshot_tx,
                })
                .await;

            if let Err(e) = oneshot_rx.await {
                console_log!("Error from oneshot_rx: {e:?}");
                return;
            }

            let _ = command_tx
                .send(command::Command::SendMessage(message::Message::FileAd(
                    file_name.clone(),
                )))
                .await;
        });

        self.messages.push_back((
            Color32::LIGHT_BLUE,
            format!("{}: File {}", self.peer_id, self.file),
        ));
        self.file.clear();
    }
}

impl eframe::App for MainApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                event::Event::Msg(message) => match message {
                    message::Message::Text {
                        source_peer_id,
                        text,
                    } => self
                        .messages
                        .push_back((Color32::GREEN, format!("{source_peer_id}: {text}"))),
                    message::Message::File { file, .. } => self.messages.push_back((
                        Color32::GREEN,
                        format!(
                            "FROM_UNKNOWN_TODO: File {}",
                            String::from_utf8_lossy(&file).to_string()
                        ),
                    )),
                    _ => {
                        console_log!("Unhandled message: {:?}", message);
                    }
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

                let file_text_resp = ui.text_edit_singleline(&mut self.file);
                if file_text_resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    if self.connected {
                        self.send_file();
                    } else {
                        console_log!("not connected to any peer");
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
