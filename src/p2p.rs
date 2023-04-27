use std::{
    collections::{hash_map::DefaultHasher, HashMap, HashSet},
    error::Error,
    hash::{Hash, Hasher},
    iter,
    path::PathBuf,
    time::Duration,
};

use async_std::io;
use async_trait::async_trait;
use futures::{
    channel::{mpsc, oneshot},
    select, AsyncRead, AsyncWrite, AsyncWriteExt, FutureExt, SinkExt, StreamExt,
};
use libp2p::{
    core::upgrade::{read_length_prefixed, write_length_prefixed, Version},
    gossipsub, identity,
    kad::{store::MemoryStore, GetProvidersOk, Kademlia, KademliaEvent, QueryId, QueryResult},
    mdns, noise,
    request_response::{self, ProtocolName, ProtocolSupport, RequestId, ResponseChannel},
    swarm::{NetworkBehaviour, SwarmBuilder, SwarmEvent},
    tcp, yamux, PeerId, Swarm, Transport,
};
use serde::{Deserialize, Serialize};

pub struct Node {
    swarm: Swarm<JiriBehaviour>,
    topic: gossipsub::IdentTopic,
    command_sender: mpsc::Sender<Command>,
    command_receiver: mpsc::Receiver<Command>,
    message_sender: async_channel::Sender<Message>,
    pending_start_providing: HashMap<QueryId, oneshot::Sender<()>>,
    pending_get_providers: HashSet<QueryId>,
    pending_request_file: HashMap<String, HashSet<RequestId>>,
}

impl Node {
    pub fn new() -> Result<
        (
            Self,
            mpsc::Sender<Command>,
            async_channel::Receiver<Message>,
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
                            Message::Text(_) => self.message_sender.send(msg).await?,
                            Message::FileName(file_name) => {
                                self.command_sender.feed(Command::GetFileProviders { file_name }).await?;
                            },
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
                                command_sender.feed(Command::RequestFile { file_name, peer }).await
                            }.boxed()
                        });

                        // Wait until at least one Command::RequestFile feeding is done
                        futures::future::select_ok(requests)
                            .await
                            .map_err(|_| "Failed to feed Command::RequestFile to any peers")?;
                    },
                    SwarmEvent::Behaviour(JiriBehaviourEvent::RequestResponse(
                        request_response::Event::Message { message, .. }
                    )) => match message {
                        request_response::Message::Request { request, channel, .. } => {
                            let path = PathBuf::from(request.0.clone()); //TODO: manage path properly
                            let file: Vec<u8> = std::fs::read(&path)?;
                            self.command_sender.feed(Command::ResponseFile { file_name: request.0, file, channel: channel }).await?
                        }
                        request_response::Message::Response { request_id, response } => {
                            log::debug!("FileResponse received: request_id:{request_id}");
                            if let Some(_) = self.pending_request_file.remove(&response.file_name) {
                                log::info!("File {} received: {:?}", response.file_name, response.file);
                            }
                        }
                    }
                    SwarmEvent::Behaviour(event) => log::info!("Event received: {event:?}"),
                    _ => {}
                },
            }
        }
    }

    fn handle_command(&mut self, command: Command) -> Result<(), Box<dyn Error>> {
        match command {
            Command::SendMessage(msg) => {
                if let Err(e) = self
                    .swarm
                    .behaviour_mut()
                    .gossipsub
                    .publish(self.topic.clone(), serde_json::to_vec(&msg)?)
                {
                    log::error!("Failed to publish: {e:?}");
                }
            }
            Command::StartFileProviding { file_name, sender } => {
                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .start_providing(file_name.into_bytes().into())?;
                self.pending_start_providing.insert(query_id, sender);
            }
            Command::GetFileProviders { file_name } => {
                let query_id = self
                    .swarm
                    .behaviour_mut()
                    .kademlia
                    .get_providers(file_name.into_bytes().into());
                self.pending_get_providers.insert(query_id);
            }
            Command::RequestFile { file_name, peer } => {
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
            Command::ResponseFile {
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

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "JiriBehaviourEvent")]
struct JiriBehaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::async_io::Behaviour,
    kademlia: Kademlia<MemoryStore>,
    request_response: request_response::Behaviour<FileExchangeCodec>,
}

#[derive(Debug)]
enum JiriBehaviourEvent {
    Gossipsub(gossipsub::Event),
    Mdns(mdns::Event),
    Kademlia(KademliaEvent),
    RequestResponse(request_response::Event<FileRequest, FileResponse>),
}

impl From<gossipsub::Event> for JiriBehaviourEvent {
    fn from(event: gossipsub::Event) -> Self {
        JiriBehaviourEvent::Gossipsub(event)
    }
}

impl From<mdns::Event> for JiriBehaviourEvent {
    fn from(event: mdns::Event) -> Self {
        JiriBehaviourEvent::Mdns(event)
    }
}

impl From<KademliaEvent> for JiriBehaviourEvent {
    fn from(event: KademliaEvent) -> Self {
        JiriBehaviourEvent::Kademlia(event)
    }
}

impl From<request_response::Event<FileRequest, FileResponse>> for JiriBehaviourEvent {
    fn from(event: request_response::Event<FileRequest, FileResponse>) -> Self {
        JiriBehaviourEvent::RequestResponse(event)
    }
}

impl JiriBehaviour {
    fn new(
        id_keys: identity::Keypair,
        peer_id: PeerId,
        topic: &gossipsub::IdentTopic,
    ) -> Result<Self, Box<dyn Error>> {
        let message_id_fn = |message: &gossipsub::Message| {
            let mut hasher = DefaultHasher::new();
            message.data.hash(&mut hasher);
            gossipsub::MessageId::from(hasher.finish().to_string())
        };
        let mut gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(id_keys),
            gossipsub::ConfigBuilder::default()
                .heartbeat_interval(Duration::from_secs(3))
                .validation_mode(gossipsub::ValidationMode::Strict)
                .message_id_fn(message_id_fn)
                .build()?,
        )?;
        gossipsub.subscribe(topic)?;

        let mdns = mdns::async_io::Behaviour::new(mdns::Config::default(), peer_id)?;

        let kademlia = Kademlia::new(peer_id, MemoryStore::new(peer_id));

        let request_response = request_response::Behaviour::new(
            FileExchangeCodec(),
            iter::once((FileExchangeProtocol(), ProtocolSupport::Full)),
            Default::default(),
        );

        Ok(Self {
            gossipsub,
            mdns,
            kademlia,
            request_response,
        })
    }
}

#[derive(Debug, Clone)]
struct FileExchangeProtocol();
#[derive(Clone)]
struct FileExchangeCodec();
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileRequest(String);
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileResponse {
    file_name: String,
    file: Vec<u8>,
}

impl ProtocolName for FileExchangeProtocol {
    fn protocol_name(&self) -> &[u8] {
        "/jiri-file-exchange/1".as_bytes()
    }
}

#[async_trait]
impl request_response::Codec for FileExchangeCodec {
    type Protocol = FileExchangeProtocol;
    type Request = FileRequest;
    type Response = FileResponse;

    async fn read_request<T>(
        &mut self,
        _: &FileExchangeProtocol,
        io: &mut T,
    ) -> io::Result<Self::Request>
    where
        T: AsyncRead + Unpin + Send,
    {
        let vec = read_length_prefixed(io, 1_000_000).await?;
        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        Ok(FileRequest(String::from_utf8(vec).unwrap()))
    }

    async fn read_response<T>(
        &mut self,
        _: &FileExchangeProtocol,
        io: &mut T,
    ) -> io::Result<Self::Response>
    where
        T: AsyncRead + Unpin + Send,
    {
        let vec = read_length_prefixed(io, 1_000_000).await?;
        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        let file_name = String::from_utf8(vec).unwrap();

        let vec = read_length_prefixed(io, 536_870_912).await?;
        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        Ok(FileResponse {
            file_name,
            file: vec,
        })
    }

    async fn write_request<T>(
        &mut self,
        _: &FileExchangeProtocol,
        io: &mut T,
        FileRequest(file_name): FileRequest,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, file_name).await?;
        io.close().await?;

        Ok(())
    }

    async fn write_response<T>(
        &mut self,
        _: &FileExchangeProtocol,
        io: &mut T,
        FileResponse { file_name, file }: FileResponse,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, file_name).await?;
        write_length_prefixed(io, file).await?;
        io.close().await?;

        Ok(())
    }
}

#[derive(Debug)]
pub enum Command {
    SendMessage(Message),
    StartFileProviding {
        file_name: String,
        sender: oneshot::Sender<()>,
    },
    GetFileProviders {
        file_name: String,
    },
    RequestFile {
        file_name: String,
        peer: PeerId,
    },
    ResponseFile {
        file_name: String,
        file: Vec<u8>,
        channel: ResponseChannel<FileResponse>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    Text(String),
    FileName(String),
}
