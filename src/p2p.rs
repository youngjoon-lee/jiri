mod behaviour;
pub mod command;
mod file_exchange;
pub mod message;

use std::{
    collections::{HashMap, HashSet},
    env,
    error::Error,
    fs::{self, File},
    io::Write,
    path::PathBuf,
    process,
    time::{SystemTime, UNIX_EPOCH},
};

use async_std::io;
use futures::{
    channel::{mpsc, oneshot},
    select, FutureExt, SinkExt, StreamExt,
};
use libp2p::{
    core::upgrade::Version,
    gossipsub, identity,
    kad::{GetProvidersOk, KademliaEvent, QueryId, QueryResult},
    mdns, noise,
    request_response::{self, RequestId},
    swarm::{SwarmBuilder, SwarmEvent},
    tcp, yamux, Swarm, Transport,
};

use crate::p2p::behaviour::{JiriBehaviour, JiriBehaviourEvent};

use self::file_exchange::{FileRequest, FileResponse};

pub struct Node {
    swarm: Swarm<JiriBehaviour>,
    topic: gossipsub::IdentTopic,
    command_sender: mpsc::Sender<command::Command>,
    command_receiver: mpsc::Receiver<command::Command>,
    message_sender: async_channel::Sender<message::Message>,
    pending_start_providing: HashMap<QueryId, oneshot::Sender<()>>,
    pending_get_providers: HashSet<QueryId>,
    pending_request_file: HashMap<String, HashSet<RequestId>>,
    tmp_dir: PathBuf,
}

impl Node {
    pub fn new() -> Result<
        (
            Self,
            mpsc::Sender<command::Command>,
            async_channel::Receiver<message::Message>,
        ),
        Box<dyn Error>,
    > {
        let id_keys = identity::Keypair::generate_ed25519();
        let peer_id = id_keys.public().to_peer_id();
        log::info!("Peer {peer_id} generated");

        let transport = tcp::async_io::Transport::default()
            .upgrade(Version::V1Lazy)
            .authenticate(noise::NoiseAuthenticated::xx(&id_keys)?)
            .multiplex(yamux::YamuxConfig::default())
            .boxed();

        let topic = gossipsub::IdentTopic::new("jiri-chat");
        let behaviour = JiriBehaviour::new(id_keys, peer_id, &topic)?;

        let mut swarm =
            SwarmBuilder::with_async_std_executor(transport, behaviour, peer_id).build();

        swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

        let (command_sender, command_receiver) = mpsc::channel(0);
        let (message_sender, message_receiver) = async_channel::unbounded();

        let tmp_dir = env::temp_dir().join(format!(
            "jiri.{}.{}",
            process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
        ));
        fs::create_dir(&tmp_dir)?;

        Ok((
            Node {
                swarm,
                topic,
                command_sender: command_sender.clone(),
                command_receiver,
                message_sender,
                pending_start_providing: Default::default(),
                pending_get_providers: Default::default(),
                pending_request_file: Default::default(),
                tmp_dir,
            },
            command_sender.clone(),
            message_receiver,
        ))
    }

    pub async fn run(mut self) -> Result<(), Box<dyn Error>> {
        loop {
            select! {
                command = self.command_receiver.select_next_some() => {
                    self.handle_command(command)?
                },
                event = self.swarm.select_next_some() => match event {
                    SwarmEvent::NewListenAddr { address, .. } => log::info!("Listening on {address:?}"),
                    SwarmEvent::Behaviour(JiriBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer_id, peer_addr) in list {
                            log::info!("mDNS discovered a new peer: {peer_id}");
                            self.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                            self.swarm.behaviour_mut().kademlia.add_address(&peer_id, peer_addr);
                        }
                    },
                    SwarmEvent::Behaviour(JiriBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                        for (peer_id, _) in list {
                            log::info!("mDNS found an expired peer: {peer_id}");
                            self.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                            self.swarm.behaviour_mut().kademlia.remove_peer(&peer_id);
                        }
                    },
                    SwarmEvent::Behaviour(JiriBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                        propagation_source,
                        message_id,
                        message
                    })) => {
                        let msg = serde_json::from_slice(&message.data)?;
                        log::info!("Got message: {:?} with ID:{message_id} from peer:{propagation_source}", msg);
                        match msg {
                            message::Message::Text(_) => self.message_sender.send(msg).await?,
                            message::Message::FileAd(file_name) => {
                                self.command_sender.feed(command::Command::GetFileProviders { file_name }).await?;
                            },
                            _ => {},
                        }
                    },
                    SwarmEvent::Behaviour(JiriBehaviourEvent::Kademlia(
                        KademliaEvent::OutboundQueryProgressed { id, result: QueryResult::StartProviding(_), .. }
                    )) => {
                        if let Some(sender) = self.pending_start_providing.remove(&id) {
                            if let Err(e) = sender.send(()) {
                                log::error!("failed to send signal that start_providing was completed: query_id:{id:?}, err:{e:?}");
                            }
                        } else {
                            log::error!("failed to find query {id:?} from pending_start_providing");
                        }
                    },
                    SwarmEvent::Behaviour(JiriBehaviourEvent::Kademlia(
                        KademliaEvent::OutboundQueryProgressed { id, result: QueryResult::GetProviders(Ok(GetProvidersOk::FoundProviders { key, providers })), .. }
                    )) => {
                        if !self.pending_get_providers.remove(&id) {
                            log::warn!("get_providers_progressed event has been already handled. skipping this duplicate event...");
                            continue;
                        }

                        // Finish the query. We are only interested in the first result.
                        self.swarm.behaviour_mut().kademlia.query_mut(&id).unwrap().finish();

                        let file_name = String::from_utf8(key.to_vec())?;

                        let requests = providers.into_iter().map(|peer| {
                            let mut command_sender = self.command_sender.clone();
                            let file_name = file_name.clone();
                            async move {
                                command_sender.feed(command::Command::RequestFile { file_name, peer }).await
                            }.boxed()
                        });

                        // Wait until at least one command::Command::RequestFile feeding is done
                        futures::future::select_ok(requests)
                            .await
                            .map_err(|_| "Failed to feed command::Command::RequestFile to any peers")?;
                    },
                    SwarmEvent::Behaviour(JiriBehaviourEvent::RequestResponse(
                        request_response::Event::Message { message, .. }
                    )) => match message {
                        request_response::Message::Request { request, channel, .. } => {
                            let file: Vec<u8> = std::fs::read(&self.tmp_dir.join(request.0.clone()))?;
                            self.command_sender.feed(command::Command::ResponseFile { file_name: request.0, file, channel: channel }).await?
                        }
                        request_response::Message::Response { request_id, response } => {
                            log::debug!("FileResponse received: request_id:{request_id}");
                            if let Some(_) = self.pending_request_file.remove(&response.file_name) {
                                log::info!("File {} received: {:?}", response.file_name, response.file);
                                self.message_sender.send(message::Message::File { file_name: response.file_name, file: response.file }).await?;
                            }
                        }
                    }
                    SwarmEvent::Behaviour(event) => log::info!("Event received: {event:?}"),
                    _ => {}
                },
            }
        }
    }

    fn handle_command(&mut self, command: command::Command) -> Result<(), Box<dyn Error>> {
        match command {
            command::Command::SendMessage(msg) => {
                if let Err(e) = self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(self.topic.clone(), serde_json::to_vec(&msg)?)
                {
                    log::error!("Failed to publish: {e:?}");
                }
            }
            command::Command::StartFileProviding {
                file_name,
                file,
                sender,
            } => {
                if let Err(e) = self.create_tmp_file(file_name.clone(), file) {
                    log::error!("failed to create tmp file: {e:?}");
                    return Err(Box::from(e));
                }

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

    fn create_tmp_file(&self, file_name: String, file: Vec<u8>) -> io::Result<()> {
        let path = self.tmp_dir.join(file_name.clone());
        File::create(path)?.write(&file)?;
        Ok(())
    }
}
