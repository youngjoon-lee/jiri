use std::error::Error;

use libp2p::{
    floodsub::{self, Floodsub},
    identity,
    swarm::{keep_alive, NetworkBehaviour},
};

#[cfg(not(feature = "web"))]
use super::file_exchange::{FileExchangeCodec, FileExchangeProtocol};
#[cfg(not(feature = "web"))]
use libp2p::{
    gossipsub,
    kad::{store::MemoryStore, Kademlia},
    mdns,
    request_response::{self, ProtocolSupport},
    PeerId,
};
#[cfg(not(feature = "web"))]
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    iter,
    time::Duration,
};

#[derive(NetworkBehaviour)]
pub struct JiriBehaviour {
    pub keep_alive: keep_alive::Behaviour,
    pub floodsub: Floodsub,

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
        floodsub_topic: floodsub::Topic,
        #[cfg(not(feature = "web"))] gossipsub_topic: &gossipsub::IdentTopic,
    ) -> Result<Self, Box<dyn Error>> {
        let peer_id = id_keys.public().to_peer_id();

        let mut floodsub = Floodsub::new(peer_id);
        floodsub.subscribe(floodsub_topic);

        #[cfg(not(feature = "web"))]
        let (gossipsub, mdns, kademlia, request_response) =
            JiriBehaviour::new_standalone(id_keys, peer_id, gossipsub_topic)?;

        Ok(Self {
            keep_alive: keep_alive::Behaviour::default(),
            floodsub,

            #[cfg(not(feature = "web"))]
            gossipsub,
            #[cfg(not(feature = "web"))]
            mdns,
            #[cfg(not(feature = "web"))]
            kademlia,
            #[cfg(not(feature = "web"))]
            request_response,
        })
    }

    #[cfg(not(feature = "web"))]
    fn new_standalone(
        id_keys: identity::Keypair,
        peer_id: PeerId,
        gossipsub_topic: &gossipsub::IdentTopic,
    ) -> Result<
        (
            gossipsub::Behaviour,
            mdns::tokio::Behaviour,
            Kademlia<MemoryStore>,
            request_response::Behaviour<FileExchangeCodec>,
        ),
        Box<dyn Error>,
    > {
        let mut gossipsub = gossipsub::Behaviour::new(
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
        )?;
        gossipsub.subscribe(gossipsub_topic)?;

        Ok((
            gossipsub,
            mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id)?,
            Kademlia::new(peer_id, MemoryStore::new(peer_id)),
            request_response::Behaviour::new(
                FileExchangeCodec(),
                iter::once((FileExchangeProtocol(), ProtocolSupport::Full)),
                Default::default(),
            ),
        ))
    }
}
