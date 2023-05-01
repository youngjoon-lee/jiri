use std::{
    collections::{hash_map::DefaultHasher, VecDeque},
    hash::{Hash, Hasher},
    time::Duration,
};

use egui::{Color32, RichText};
use futures::prelude::*;
use libp2p::{
    core::upgrade::Version,
    floodsub::{self, Floodsub, FloodsubEvent},
    identity, noise,
    swarm::{keep_alive, NetworkBehaviour, SwarmBuilder, SwarmEvent},
    yamux, Multiaddr, PeerId, Transport,
};
use libp2p_websys_transport::WebsocketTransport;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
}

fn main() {
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
    event_rx: async_channel::Receiver<Event>,
    command_tx: async_channel::Sender<Command>,
    messages: VecDeque<(Color32, String)>,
    connected: bool,
    text: String,
}

impl MainApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());

        let (event_tx, event_rx) = async_channel::bounded(64);
        let (command_tx, command_rx) = async_channel::bounded(64);

        spawn_local(libp2p_service(command_rx, event_tx));

        Self {
            event_rx,
            command_tx,
            messages: Default::default(),
            connected: false,
            text: "/ip4/127.0.0.1/tcp/8081/ws".to_string(),
        }
    }

    fn send_command(&self, command: Command) {
        let tx = self.command_tx.clone();
        spawn_local(async move {
            let _ = tx.send(command).await;
        });
    }

    fn send_chat(&mut self) {
        self.send_command(Command::Chat(self.text.clone()));
        self.messages
            .push_back((Color32::LIGHT_BLUE, format!("{: >20}", self.text)));
        self.text.clear();
    }
}

impl eframe::App for MainApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                Event::Msg(text) => {
                    self.messages.push_back((Color32::GREEN, text));
                }
                Event::Connected(peer_id) => {
                    self.connected = true;
                    self.text.clear();
                    self.messages
                        .push_back((Color32::YELLOW, format!("Connected to {peer_id}")));
                }
                Event::Disconnected(peer_id) => {
                    self.connected = true;
                    self.messages
                        .push_back((Color32::YELLOW, format!("Disconnected to {peer_id}")));
                }
                Event::Error(e) => {
                    self.connected = false;
                    self.messages
                        .push_back((Color32::RED, format!("Error: {e}")));
                }
            }
        }

        // Render command panel
        egui::TopBottomPanel::bottom("command").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if self.connected {
                    if ui.button("Chat").clicked() {
                        self.send_chat();
                    }
                } else if ui.button("Connect").clicked() {
                    if let Ok(address) = self.text.parse::<Multiaddr>() {
                        self.send_command(Command::Dial(address));
                    } else {
                        self.messages.push_back((
                            Color32::RED,
                            format!("Invalid multiaddr {}", self.text.clone()),
                        ));
                    }
                }

                let resp = ui.text_edit_singleline(&mut self.text);
                if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    self.send_chat();
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

enum Command {
    Dial(Multiaddr),
    Chat(String),
}

enum Event {
    Connected(PeerId),
    Disconnected(PeerId),
    Msg(String),
    Error(String),
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    Text(String),
    FileAd(String),
    File { file_name: String, file: Vec<u8> },
}

async fn libp2p_service(
    mut command_rx: async_channel::Receiver<Command>,
    event_tx: async_channel::Sender<Event>,
) {
    // Create the websocket transport
    let local_key = identity::Keypair::generate_ed25519();
    let transport = WebsocketTransport::default()
        .upgrade(Version::V1Lazy)
        .authenticate(noise::NoiseAuthenticated::xx(&local_key).unwrap())
        .multiplex(yamux::YamuxConfig::default())
        .boxed();

    // Create a behaviour to receive floodsub messages and keep alive connection
    #[derive(NetworkBehaviour)]
    struct JiriWebBehaviour {
        keep_alive: keep_alive::Behaviour,
        floodsub: Floodsub,
    }

    let topic = floodsub::Topic::new("jiri-chat-floodsub");

    let mut swarm = {
        let local_peer_id = PeerId::from(local_key.public());
        let mut behaviour = JiriWebBehaviour {
            keep_alive: keep_alive::Behaviour::default(),
            floodsub: Floodsub::new(local_peer_id),
        };
        behaviour.floodsub.subscribe(topic.clone());

        SwarmBuilder::with_wasm_executor(transport, behaviour, local_peer_id).build()
    };

    loop {
        futures::select! {
            command = command_rx.select_next_some() => match command {
                Command::Dial(addr) => {
                    if let Err(e) = swarm.dial(addr) {
                        let _ = event_tx.send(Event::Error(e.to_string())).await;
                    }
                }
                Command::Chat(text) => {
                    let message = Message::Text(text);
                    swarm.behaviour_mut().floodsub.publish(topic.clone(), serde_json::to_vec(&message).unwrap());
                }
            },
            event = swarm.select_next_some() => match event {
                SwarmEvent::Behaviour(JiriWebBehaviourEvent::Floodsub(FloodsubEvent::Message(message))) => {
                    let msg = serde_json::from_slice(&message.data).unwrap();
                    match msg {
                        Message::Text(text) =>  {
                            let event = Event::Msg(text);
                            let _ = event_tx.send(event).await;
                        }
                        //TODO: handle FileAd and File as well
                        _ => {}
                    }
                },
                SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                    swarm.behaviour_mut().floodsub.add_node_to_partial_view(peer_id);
                    let _ = event_tx.send(Event::Connected(peer_id)).await;
                }
                SwarmEvent::ConnectionClosed { peer_id, .. } => {
                    swarm.behaviour_mut().floodsub.remove_node_from_partial_view(&peer_id);
                    let _ = event_tx.send(Event::Disconnected(peer_id)).await;
                }
                SwarmEvent::OutgoingConnectionError { error, .. } => {
                    let _ = event_tx.send(Event::Error(error.to_string())).await;
                }
                _ => {} //TODO: console_log!
            }
        }
    }
}
