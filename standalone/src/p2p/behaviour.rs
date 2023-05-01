use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    iter,
    time::Duration,
};

use libp2p::{
    floodsub::{self, Floodsub},
    gossipsub, identity,
    kad::{store::MemoryStore, Kademlia},
    mdns,
    request_response::{self, ProtocolSupport},
    swarm::{keep_alive, NetworkBehaviour},
    PeerId,
};

use super::file_exchange::{FileExchangeCodec, FileExchangeProtocol};

#[derive(NetworkBehaviour)]
pub struct JiriBehaviour {
    pub gossipsub: gossipsub::Behaviour,
    pub floodsub: Floodsub,
    //TODO: Understand what keep_alive actually is
    pub keep_alive: keep_alive::Behaviour,
    pub mdns: mdns::async_io::Behaviour,
    pub kademlia: Kademlia<MemoryStore>,
    pub request_response: request_response::Behaviour<FileExchangeCodec>,
}

impl JiriBehaviour {
    pub fn new(
        id_keys: identity::Keypair,
        peer_id: PeerId,
        gossipsub_topic: &gossipsub::IdentTopic,
        floodsub_topic: floodsub::Topic,
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
        gossipsub.subscribe(gossipsub_topic)?;

        let mut floodsub = Floodsub::new(peer_id);
        floodsub.subscribe(floodsub_topic);

        let keep_alive = keep_alive::Behaviour::default();

        let mdns = mdns::async_io::Behaviour::new(mdns::Config::default(), peer_id)?;

        let kademlia = Kademlia::new(peer_id, MemoryStore::new(peer_id));

        let request_response = request_response::Behaviour::new(
            FileExchangeCodec(),
            iter::once((FileExchangeProtocol(), ProtocolSupport::Full)),
            Default::default(),
        );

        Ok(Self {
            gossipsub,
            floodsub,
            keep_alive,
            mdns,
            kademlia,
            request_response,
        })
    }
}
