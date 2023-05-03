mod behaviour;
pub mod command;
pub mod event;
mod file_exchange;
pub mod message;
mod transport;

use crate::p2p::{
    behaviour::{JiriBehaviour, JiriBehaviourEvent},
    event::Event,
    file_exchange::{FileRequest, FileResponse},
    message::Message,
};
use futures::{
    channel::{mpsc, oneshot},
    select, FutureExt, SinkExt, StreamExt,
};
use libp2p::{
    floodsub::{self, FloodsubEvent},
    identity,
    kad::{GetProvidersOk, KademliaEvent, QueryId, QueryResult},
    request_response::RequestId,
    request_response::{self, ResponseChannel},
    swarm::{SwarmBuilder, SwarmEvent},
    PeerId, Swarm,
};
use std::{
    collections::{HashMap, HashSet},
    error::Error,
};

#[cfg(not(feature = "web"))]
use libp2p::{gossipsub, mdns, Multiaddr};
#[cfg(not(feature = "web"))]
use std::net::SocketAddrV4;

pub struct Core {
    pub peer_id: PeerId,
    swarm: Swarm<JiriBehaviour>,
    floodsub_topic: floodsub::Topic,
    command_tx: mpsc::UnboundedSender<command::Command>,
    command_rx: mpsc::UnboundedReceiver<command::Command>,
    event_tx: async_channel::Sender<Event>,
    pending_request_file: HashMap<String, HashSet<RequestId>>,
    pending_start_providing: HashMap<QueryId, oneshot::Sender<()>>,
    pending_get_providers: HashSet<QueryId>,
    file_store: HashMap<String, Vec<u8>>,

    #[cfg(not(feature = "web"))]
    gossipsub_topic: gossipsub::IdentTopic,
}

impl Core {
    pub fn new() -> Result<
        (
            Self,
            mpsc::UnboundedSender<command::Command>,
            async_channel::Receiver<Event>,
        ),
        Box<dyn Error>,
    > {
        let id_keys = identity::Keypair::generate_ed25519();
        let peer_id = id_keys.public().to_peer_id();
        log::info!("Peer {peer_id} generated");

        let floodsub_topic = floodsub::Topic::new("jiri-chat-floodsub");
        #[cfg(not(feature = "web"))]
        let gossipsub_topic = gossipsub::IdentTopic::new("jiri-chat");

        #[cfg(not(feature = "web"))]
        let behaviour =
            JiriBehaviour::new(id_keys.clone(), floodsub_topic.clone(), &gossipsub_topic)?;
        #[cfg(feature = "web")]
        let behaviour = JiriBehaviour::new(id_keys.clone(), floodsub_topic.clone())?;

        #[cfg(not(feature = "web"))]
        let swarm = SwarmBuilder::with_tokio_executor(
            transport::create_transport(&id_keys)?,
            behaviour,
            peer_id,
        )
        .build();

        #[cfg(feature = "web")]
        let swarm = SwarmBuilder::with_wasm_executor(
            transport::create_transport(&id_keys)?,
            behaviour,
            peer_id,
        )
        .build();

        let (command_tx, command_rx) = mpsc::unbounded();
        let (event_tx, event_rx) = async_channel::unbounded();

        Ok((
            Core {
                peer_id,
                swarm,
                floodsub_topic,
                command_tx: command_tx.clone(),
                command_rx,
                event_tx,
                pending_request_file: Default::default(),
                pending_start_providing: Default::default(),
                pending_get_providers: Default::default(),
                file_store: Default::default(),

                #[cfg(not(feature = "web"))]
                gossipsub_topic,
            },
            command_tx.clone(),
            event_rx,
        ))
    }

    #[cfg(not(feature = "web"))]
    pub async fn run(
        mut self,
        tcp_laddr: &String,
        ws_laddr: &String,
    ) -> Result<(), Box<dyn Error>> {
        let tcp_addr = tcp_laddr.parse::<SocketAddrV4>()?;
        self.swarm.listen_on(
            format!("/ip4/{}/tcp/{}", tcp_addr.ip().to_string(), tcp_addr.port()).parse()?,
        )?;

        let ws_addr = ws_laddr.parse::<SocketAddrV4>()?;
        self.swarm.listen_on(
            format!(
                "/ip4/{}/tcp/{}/ws",
                ws_addr.ip().to_string(),
                ws_addr.port()
            )
            .parse()?,
        )?;

        self.start_loop().await
    }

    #[cfg(feature = "web")]
    pub async fn run(self) -> Result<(), Box<dyn Error>> {
        self.start_loop().await
    }

    async fn start_loop(mut self) -> Result<(), Box<dyn Error>> {
        loop {
            select! {
                command = self.command_rx.select_next_some() => {
                    self.handle_command(command)?
                },
                event = self.swarm.select_next_some() => {
                    self.handle_event(event).await?
                },
            }
        }
    }

    async fn handle_event<T>(
        &mut self,
        event: SwarmEvent<JiriBehaviourEvent, T>,
    ) -> Result<(), Box<dyn Error>> {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => log::info!("Listening on {address:?}"),
            SwarmEvent::ConnectionEstablished {
                peer_id, endpoint, ..
            } => {
                self.swarm
                    .behaviour_mut()
                    .kademlia
                    .add_address(&peer_id, endpoint.get_remote_address().clone());
                self.swarm
                    .behaviour_mut()
                    .floodsub
                    .add_node_to_partial_view(peer_id);
                self.event_tx.send(Event::Connected(peer_id)).await?;
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                self.swarm.behaviour_mut().kademlia.remove_peer(&peer_id);
                self.swarm
                    .behaviour_mut()
                    .floodsub
                    .remove_node_from_partial_view(&peer_id);
                self.event_tx.send(Event::Disconnected(peer_id)).await?;
            }
            SwarmEvent::OutgoingConnectionError { error, .. } => {
                log::error!("OutgoingConnectionError: {error}");
                self.event_tx.send(Event::Error(error.to_string())).await?;
            }
            SwarmEvent::Behaviour(JiriBehaviourEvent::Floodsub(FloodsubEvent::Message(
                message,
            ))) => self.handle_message(message.source, message.data).await?,
            SwarmEvent::Behaviour(JiriBehaviourEvent::Kademlia(
                KademliaEvent::OutboundQueryProgressed {
                    id,
                    result: QueryResult::StartProviding(_),
                    ..
                },
            )) => self.handle_start_providing_progressed(id),
            SwarmEvent::Behaviour(JiriBehaviourEvent::Kademlia(
                KademliaEvent::OutboundQueryProgressed {
                    id,
                    result:
                        QueryResult::GetProviders(Ok(GetProvidersOk::FoundProviders { key, providers })),
                    ..
                },
            )) => {
                self.handle_get_providers_progressed(id, key.to_vec(), providers)
                    .await?
            }
            SwarmEvent::Behaviour(JiriBehaviourEvent::RequestResponse(
                request_response::Event::Message { message, .. },
            )) => match message {
                request_response::Message::Request {
                    request, channel, ..
                } => self.handle_file_request_event(request, channel).await?,
                request_response::Message::Response {
                    request_id,
                    response,
                } => {
                    self.handle_file_response_event(request_id, response)
                        .await?
                }
            },
            _ => {
                #[cfg(not(feature = "web"))]
                self.handle_standalone_event(event).await?;
                #[cfg(feature = "web")]
                if let SwarmEvent::Behaviour(ev) = event {
                    log::info!("Unrecognized event received: {ev:?}");
                }
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "web"))]
    async fn handle_standalone_event<T>(
        &mut self,
        event: SwarmEvent<JiriBehaviourEvent, T>,
    ) -> Result<(), Box<dyn Error>> {
        match event {
            SwarmEvent::Behaviour(JiriBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                for (peer_id, peer_addr) in list {
                    self.handle_peer_discovered(peer_id, peer_addr);
                }
            }
            SwarmEvent::Behaviour(JiriBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                for (peer_id, _) in list {
                    self.handle_peer_expired(peer_id);
                }
            }
            SwarmEvent::Behaviour(JiriBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                propagation_source,
                message,
                ..
            })) => {
                self.handle_message(propagation_source, message.data)
                    .await?
            }
            _ => {
                if let SwarmEvent::Behaviour(ev) = event {
                    log::info!("Unrecognized event received: {ev:?}");
                }
            }
        }

        Ok(())
    }

    #[cfg(not(feature = "web"))]
    fn handle_peer_discovered(&mut self, peer_id: PeerId, peer_addr: Multiaddr) {
        log::info!("mDNS discovered a new peer: {peer_id}");
        self.swarm
            .behaviour_mut()
            .gossipsub
            .add_explicit_peer(&peer_id);
        self.swarm
            .behaviour_mut()
            .kademlia
            .add_address(&peer_id, peer_addr);
        self.swarm
            .behaviour_mut()
            .floodsub
            .add_node_to_partial_view(peer_id);
    }

    #[cfg(not(feature = "web"))]
    fn handle_peer_expired(&mut self, peer_id: PeerId) {
        log::info!("mDNS found an expired peer: {peer_id}");
        self.swarm
            .behaviour_mut()
            .gossipsub
            .remove_explicit_peer(&peer_id);
        self.swarm.behaviour_mut().kademlia.remove_peer(&peer_id);
        self.swarm
            .behaviour_mut()
            .floodsub
            .remove_node_from_partial_view(&peer_id);
    }

    async fn handle_message(
        &mut self,
        propagation_source: PeerId,
        message_data: Vec<u8>,
    ) -> Result<(), Box<dyn Error>> {
        let msg = serde_json::from_slice(&message_data)?;
        log::info!("Got message: {:?} from peer:{propagation_source}", msg);

        match msg {
            message::Message::Text {
                source_peer_id,
                text,
            } => {
                self.event_tx
                    .send(Event::Msg(Message::Text {
                        source_peer_id,
                        text,
                    }))
                    .await?
            }
            message::Message::FileAd(file_name) => {
                self.command_tx
                    .send(command::Command::GetFileProviders { file_name })
                    .await?;
            }
            _ => {}
        };

        Ok(())
    }

    fn handle_start_providing_progressed(&mut self, id: QueryId) {
        if let Some(sender) = self.pending_start_providing.remove(&id) {
            if let Err(e) = sender.send(()) {
                log::error!("failed to send signal that start_providing was completed: query_id:{id:?}, err:{e:?}");
            }
        } else {
            log::error!("failed to find query {id:?} from pending_start_providing");
        }
    }

    async fn handle_get_providers_progressed(
        &mut self,
        id: QueryId,
        key: Vec<u8>,
        providers: HashSet<PeerId>,
    ) -> Result<(), Box<dyn Error>> {
        if !self.pending_get_providers.remove(&id) {
            log::warn!("get_providers_progressed event has been already handled. skipping this duplicate event...");
            return Ok(());
        }

        // Finish the query. We are only interested in the first result.
        self.swarm
            .behaviour_mut()
            .kademlia
            .query_mut(&id)
            .unwrap()
            .finish();

        let file_name = String::from_utf8(key)?;

        let requests = providers.into_iter().map(|peer| {
            let mut command_tx = self.command_tx.clone();
            let file_name = file_name.clone();
            async move {
                command_tx
                    .send(command::Command::RequestFile { file_name, peer })
                    .await
            }
            .boxed()
        });

        // Wait until at least one command::Command::RequestFile sending is done
        futures::future::select_ok(requests)
            .await
            .map_err(|_| "Failed to send command::Command::RequestFile to any peers")?;

        Ok(())
    }

    async fn handle_file_request_event(
        &mut self,
        request: FileRequest,
        channel: ResponseChannel<FileResponse>,
    ) -> Result<(), Box<dyn Error>> {
        let Some(file) = self.file_store.get(&request.0) else {
            return Err(format!("file {} not found from store", request.0).into());
        };

        self.command_tx
            .send(command::Command::ResponseFile {
                file_name: request.0,
                file: file.clone(),
                channel,
            })
            .await?;

        Ok(())
    }

    async fn handle_file_response_event(
        &mut self,
        request_id: RequestId,
        response: FileResponse,
    ) -> Result<(), Box<dyn Error>> {
        log::debug!("FileResponse received: request_id:{request_id}");
        if let Some(_) = self.pending_request_file.remove(&response.file_name) {
            log::info!("File {} received: {:?}", response.file_name, response.file);
            self.event_tx
                .send(Event::Msg(message::Message::File {
                    file_name: response.file_name,
                    file: response.file,
                }))
                .await?;
        }

        Ok(())
    }

    fn handle_command(&mut self, command: command::Command) -> Result<(), Box<dyn Error>> {
        match command {
            command::Command::Dial(addr) => {
                if let Err(e) = self.swarm.dial(addr.clone()) {
                    // let _ = event_tx.send(Event::Error(e.to_string())).await;
                    log::error!("failed to dial to {addr:?}: {e}");
                }
            }
            command::Command::SendMessage(msg) => {
                #[cfg(not(feature = "web"))]
                if let Err(e) = self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(self.gossipsub_topic.clone(), serde_json::to_vec(&msg)?)
                {
                    log::error!("Failed to publish: {e:?}");
                }

                #[cfg(feature = "web")]
                self.swarm
                    .behaviour_mut()
                    .floodsub
                    .publish(self.floodsub_topic.clone(), serde_json::to_vec(&msg)?);

                log::trace!("self.floodsub_topic: {:?}", self.floodsub_topic);
            }
            command::Command::StartFileProviding {
                file_name,
                file,
                sender,
            } => {
                self.file_store.insert(file_name.clone(), file);
                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .start_providing(file_name.into_bytes().into())?;
                self.pending_start_providing.insert(query_id, sender);
            }
            command::Command::GetFileProviders { file_name } => {
                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .get_providers(file_name.into_bytes().into());
                self.pending_get_providers.insert(query_id);
            }
            command::Command::RequestFile { file_name, peer } => {
                //TODO: This doesn't work if the peer is in browser.
                let request_id = self
                    .swarm
                    .behaviour_mut()
                    .request_response
                    .send_request(&peer, FileRequest(file_name.clone()));

                if let Some(request_ids) = self.pending_request_file.get_mut(&file_name) {
                    request_ids.insert(request_id);
                } else {
                    let mut request_ids: HashSet<RequestId> = Default::default();
                    request_ids.insert(request_id);
                    self.pending_request_file.insert(file_name, request_ids);
                }
            }
            command::Command::ResponseFile {
                file_name,
                file,
                channel,
            } => {
                self.swarm
                    .behaviour_mut()
                    .request_response
                    .send_response(channel, FileResponse { file_name, file })
                    .expect("Connection to peer to be still open");
            }
        };

        Ok(())
    }
}
