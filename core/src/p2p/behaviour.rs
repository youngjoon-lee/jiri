use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    iter,
    time::Duration,
};

use libp2p::{
    floodsub::{self, Floodsub},
    identity,
    request_response::{self, ProtocolSupport},
    swarm::NetworkBehaviour,
    PeerId,
};

#[cfg(not(feature = "web"))]
use libp2p::{
    kad::{store::MemoryStore, Kademlia},
    mdns,
};

#[cfg(not(feature = "web"))]
use libp2p::gossipsub;

#[cfg(feature = "web")]
use libp2p::swarm::keep_alive;

use super::file_exchange::{FileExchangeCodec, FileExchangeProtocol};

#[derive(NetworkBehaviour)]
pub struct JiriBehaviour {
    pub floodsub: Floodsub,

    #[cfg(feature = "web")]
    pub keep_alive: keep_alive::Behaviour,

    #[cfg(not(feature = "web"))]
    pub gossipsub: gossipsub::Behaviour,
    #[cfg(not(feature = "web"))]
    pub mdns: mdns::tokio::Behaviour,
    #[cfg(not(feature = "web"))]
    pub kademlia: Kademlia<MemoryStore>,
    #[cfg(not(feature = "web"))]
    pub request_response: request_response::Behaviour<FileExchangeCodec>,
}

impl JiriBehaviour {
    pub fn new(
        id_keys: identity::Keypair,
        peer_id: PeerId,
        floodsub_topic: floodsub::Topic,
        #[cfg(not(feature = "web"))] gossipsub_topic: &gossipsub::IdentTopic,
    ) -> Result<Self, Box<dyn Error>> {
        let mut behaviour = Self {
            floodsub: Floodsub::new(peer_id),

            #[cfg(feature = "web")]
            keep_alive: keep_alive::Behaviour::default(),

            #[cfg(not(feature = "web"))]
            gossipsub: gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(id_keys),
                gossipsub::ConfigBuilder::default()
                    .heartbeat_interval(Duration::from_secs(3))
                    .validation_mode(gossipsub::ValidationMode::Strict)
                    .message_id_fn(|message: &gossipsub::Message| {
                        let mut hasher = DefaultHasher::new();
                        message.data.hash(&mut hasher);
                        gossipsub::MessageId::from(hasher.finish().to_string())
                    })
                    .build()?,
            )?,

            #[cfg(not(feature = "web"))]
            mdns: mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id)?,

            #[cfg(not(feature = "web"))]
            kademlia: Kademlia::new(peer_id, MemoryStore::new(peer_id)),

            #[cfg(not(feature = "web"))]
            request_response: request_response::Behaviour::new(
                FileExchangeCodec(),
                iter::once((FileExchangeProtocol(), ProtocolSupport::Full)),
                Default::default(),
            ),
        };

        behaviour.floodsub.subscribe(floodsub_topic);

        #[cfg(not(feature = "web"))]
        behaviour.gossipsub.subscribe(gossipsub_topic)?;

        Ok(behaviour)
    }
}
