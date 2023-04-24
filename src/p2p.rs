use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    time::Duration,
};

use async_std::io;
use futures::{select, AsyncBufReadExt, StreamExt};
use libp2p::{
    core::upgrade::Version,
    gossipsub, identity, mdns, noise,
    swarm::{NetworkBehaviour, SwarmBuilder, SwarmEvent},
    tcp, yamux, PeerId, Transport,
};

#[derive(Debug)]
pub struct Node;

impl Node {
    pub async fn new() -> Result<Self, Box<dyn Error>> {
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

        let mut stdin = io::BufReader::new(io::stdin()).lines().fuse();

        swarm.listen_on("/ip4/0.0.0.0/tcp/0".parse()?)?;

        loop {
            select! {
                line = stdin.select_next_some() => {
                    if let Err(e) = swarm.behaviour_mut().gossipsub.publish(topic.clone(), line?.as_bytes()) {
                        log::error!("Failed to publish: {e:?}");
                    }
                },
                event = swarm.select_next_some() => match event {
                    SwarmEvent::NewListenAddr { address, .. } => log::info!("Listening on {address:?}"),
                    SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                        for (peer_id, _) in list {
                            log::info!("mDNS discovered a new peer: {peer_id}");
                            swarm.behaviour_mut().gossipsub.add_explicit_peer(&peer_id);
                        }
                    },
                    SwarmEvent::Behaviour(BehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                        for (peer_id, _) in list {
                            log::info!("mDNS found an expired peer: {peer_id}");
                            swarm.behaviour_mut().gossipsub.remove_explicit_peer(&peer_id);
                        }
                    },
                    SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(gossipsub::Event::Message {
                        propagation_source,
                        message_id,
                        message
                    })) => {
                        log::info!("Got message: {} with ID:{message_id} from peer:{propagation_source}", String::from_utf8_lossy(&message.data));
                    },
                    SwarmEvent::Behaviour(event) => log::info!("Event received: {event:?}"),
                    _ => {}
                },
            }
        }
    }
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    gossipsub: gossipsub::Behaviour,
    mdns: mdns::async_io::Behaviour,
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

        Ok(Self { gossipsub, mdns })
    }
}
