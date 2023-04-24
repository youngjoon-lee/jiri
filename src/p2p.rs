use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    time::Duration,
};

use futures::StreamExt;
use libp2p::{
    core::upgrade::Version,
    gossipsub, identity, mdns, noise,
    swarm::{NetworkBehaviour, SwarmBuilder, SwarmEvent},
    tcp, yamux, PeerId, Transport,
};

pub async fn new() -> Result<Client, Box<dyn Error>> {
    let id_keys = identity::Keypair::generate_ed25519();
    let peer_id = id_keys.public().to_peer_id();
    log::info!("Peer {peer_id} generated");

    let transport = tcp::async_io::Transport::default()
        .upgrade(Version::V1Lazy)
        .authenticate(noise::NoiseAuthenticated::xx(&id_keys)?)
        .multiplex(yamux::YamuxConfig::default())
        .boxed();

    let behaviour = Behaviour::new(id_keys, peer_id, "jiri-chat")?;

    let mut swarm = SwarmBuilder::with_async_std_executor(transport, behaviour, peer_id).build();

    swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

    loop {
        match swarm.select_next_some().await {
            SwarmEvent::NewListenAddr { address, .. } => log::info!("Listening on {address:?}"),
            _ => {}
        }
    }
}

#[derive(Debug)]
pub struct Client;

#[derive(NetworkBehaviour)]
struct Behaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::async_io::Behaviour,
}

impl Behaviour {
    fn new(
        id_keys: identity::Keypair,
        peer_id: PeerId,
        topic: &str,
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
        gossipsub.subscribe(&gossipsub::IdentTopic::new(topic))?;

        let mdns = mdns::async_io::Behaviour::new(mdns::Config::default(), peer_id)?;

        Ok(Self { gossipsub, mdns })
    }
}
