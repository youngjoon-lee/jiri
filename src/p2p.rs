use std::{
    collections::{hash_map::DefaultHasher, HashMap},
    error::Error,
    hash::{Hash, Hasher},
    iter,
    time::Duration,
};

use async_std::io;
use async_trait::async_trait;
use futures::{
    channel::{mpsc, oneshot},
    select, AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, StreamExt,
};
use libp2p::{
    core::upgrade::{read_length_prefixed, write_length_prefixed, Version},
    gossipsub, identity,
    kad::{store::MemoryStore, Kademlia, KademliaEvent, QueryId, QueryResult},
    mdns, noise,
    request_response::{self, ProtocolName, ProtocolSupport},
    swarm::{NetworkBehaviour, SwarmBuilder, SwarmEvent},
    tcp, yamux, PeerId, Swarm, Transport,
};
use serde::{Deserialize, Serialize};

pub struct Node {
    swarm: Swarm<Behaviour>,
    topic: gossipsub::IdentTopic,
    command_receiver: mpsc::Receiver<Command>,
    message_sender: async_channel::Sender<Message>,
    pending_start_providing: HashMap<QueryId, oneshot::Sender<()>>,
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
        let behaviour = Behaviour::new(id_keys, peer_id, &topic)?;

        let mut swarm =
            SwarmBuilder::with_async_std_executor(transport, behaviour, peer_id).build();

        swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

        let (command_sender, command_receiver) = mpsc::channel(0);
        let (message_sender, message_receiver) = async_channel::unbounded();

        Ok((
            Node {
                swarm,
                topic,
                command_receiver,
                message_sender,
                pending_start_providing: Default::default(),
            },
            command_sender,
            message_receiver,
        ))
    }

    pub async fn run(mut self) -> Result<(), Box<dyn Error>> {
        let mut stdin = io::BufReader::new(io::stdin()).lines().fuse();

        loop {
            select! {
                line = stdin.select_next_some() => {
                    let msg = serde_json::to_vec(&Message::Text(line?))?;
                    if let Err(e) = self.swarm.behaviour_mut().gossipsub.publish(self.topic.clone(), msg) {
                        log::error!("Failed to publish: {e:?}");
                    }
                },
                command = self.command_receiver.select_next_some() => {
                    self.handle_command(command)?
                },
                event = self.swarm.select_next_some() => match event {
                    SwarmEvent::NewListenAddr { address, .. } => log::info!("Listening on {address:?}"),
                    SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer_id, _) in list {
                            log::info!("mDNS discovered a new peer: {peer_id}");
                            self.swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        }
                    },
                    SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                        for (peer_id, _) in list {
                            log::info!("mDNS found an expired peer: {peer_id}");
                            self.swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                        }
                    },
                    SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
                        propagation_source,
                        message_id,
                        message
                    })) => {
                        let msg = serde_json::from_slice(&message.data)?;
                        // TODO: if msg is FileName(..), request the file to the provider
                        log::info!("Got message: {:?} with ID:{message_id} from peer:{propagation_source}", msg);
                        self.message_sender.send(msg).await?;
                    },
                    SwarmEvent::Behaviour(BehaviourEvent::Kademlia(
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
                    SwarmEvent::Behaviour(BehaviourEvent::RequestResponse(
                        request_response::Event::Message { message, .. }
                    )) => {
                        //TODO: implement this
                        log::info!("message:{message:?}");
                    },
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
        };

        Ok(())
    }
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::async_io::Behaviour,
    kademlia: Kademlia<MemoryStore>,
    request_response: request_response::Behaviour<FileExchangeCodec>,
}

impl Behaviour {
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
pub struct FileResponse(Vec<u8>);

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
        let vec = read_length_prefixed(io, 536_870_912).await?;
        if vec.is_empty() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }

        Ok(FileResponse(vec))
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
        FileResponse(data): FileResponse,
    ) -> io::Result<()>
    where
        T: AsyncWrite + Unpin + Send,
    {
        write_length_prefixed(io, data).await?;
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
}

#[derive(Debug, Serialize, Deserialize)]
pub enum Message {
    Text(String),
    FileName(String),
}
