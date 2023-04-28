use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    iter,
    time::Duration,
};

use libp2p::{
    gossipsub, identity,
    kad::{store::MemoryStore, Kademlia, KademliaEvent},
    mdns,
    request_response::{self, ProtocolSupport},
    swarm::NetworkBehaviour,
    PeerId,
};

use super::file_exchange::{FileExchangeCodec, FileExchangeProtocol, FileRequest, FileResponse};

#[derive(NetworkBehaviour)]
#[behaviour(out_event = "JiriBehaviourEvent")]
pub struct JiriBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub mdns: mdns::async_io::Behaviour,
    pub kademlia: Kademlia<MemoryStore>,
    pub request_response: request_response::Behaviour<FileExchangeCodec>,
}

#[derive(Debug)]
pub enum JiriBehaviourEvent {
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
    pub fn new(
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
