mod utils;

extern crate alloc;

use std::{
    collections::hash_map::DefaultHasher,
    error::Error,
    hash::{Hash, Hasher},
    time::Duration,
};

use futures_util::StreamExt;
use libp2p::{
    core::{muxing::StreamMuxerBox, upgrade},
    gossipsub, identify, identity,
    kad::{store::MemoryStore, Kademlia, KademliaConfig},
    multiaddr::{Multiaddr, Protocol},
    noise, relay,
    swarm::{keep_alive, AddressScore, NetworkBehaviour, SwarmBuilder, SwarmEvent},
    wasm_ext::{ffi::websocket_transport, ExtTransport},
    yamux, PeerId, StreamProtocol, Swarm, Transport,
};
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

// When the `wee_alloc` feature is enabled, use `wee_alloc` as the global
// allocator.
#[cfg(feature = "wee_alloc")]
#[global_allocator]
static ALLOC: wee_alloc::WeeAlloc = wee_alloc::WeeAlloc::INIT;

// Debugging console log.
#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);
}

macro_rules! console_log {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

const KADEMLIA_PROTOCOL_NAME: &str = "/jiri/lan/kad/1.0.0";
const GOSSIPSUB_TOPIC: &str = "jiri";

#[wasm_bindgen]
pub fn start(remote_addr: String) {
    utils::set_panic_hook();

    let remote_multiaddr = remote_addr.parse::<Multiaddr>().unwrap();

    spawn_local(async {
        run(remote_multiaddr).await;
    })
}

pub async fn run(remote_multiaddr: Multiaddr) {
    let local_key = identity::Keypair::generate_ed25519();
    let local_peer_id = PeerId::from(local_key.public());
    console_log!("Local peer id: {}", local_peer_id);

    let mut swarm = create_swarm(local_key, local_peer_id).unwrap();

    // this is also a relay server
    swarm.dial(remote_multiaddr.clone()).unwrap();

    // As a relay client, learn our local public address from the relay server
    // , and enable a freshly started relay to learn its public address
    let mut learned_observed_addr = false;
    let mut told_relay_observed_addr = false;
    loop {
        match swarm.next().await.unwrap() {
            SwarmEvent::NewListenAddr { .. } => {}
            SwarmEvent::Dialing { .. } => {}
            SwarmEvent::ConnectionEstablished { .. } => {}
            SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Sent { .. })) => {
                console_log!("Told relay its public address.");
                told_relay_observed_addr = true;
            }
            SwarmEvent::Behaviour(BehaviourEvent::Identify(identify::Event::Received {
                info: identify::Info { observed_addr, .. },
                ..
            })) => {
                console_log!("Relay told us our public address: {:?}", observed_addr);
                learned_observed_addr = true;
            }
            _ => {}
        }

        if learned_observed_addr && told_relay_observed_addr {
            break;
        }
    }

    // As a relay client,
    let relay_address = remote_multiaddr.with(Protocol::P2pCircuit);
    swarm.listen_on(relay_address.clone()).unwrap();
    console_log!("Listening via relay: {relay_address}");

    loop {
        match swarm.next().await.unwrap() {
            SwarmEvent::NewListenAddr { address, .. } => {
                console_log!("Listen p2p address: {address:?}");
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                console_log!("Connected to {peer_id}");
            }
            SwarmEvent::OutgoingConnectionError { peer_id, error } => {
                console_log!("Failed to dial {peer_id:?}: {error}");
            }
            SwarmEvent::ConnectionClosed { peer_id, cause, .. } => {
                console_log!("Connection to {peer_id} closed: {cause:?}");
                swarm.behaviour_mut().kademlia.remove_peer(&peer_id);
                console_log!("Removed {peer_id} from the routing table (if it was in there).");
            }
            SwarmEvent::Behaviour(BehaviourEvent::RelayClient(
                relay::client::Event::ReservationReqAccepted { relay_peer_id, .. },
            )) => {
                console_log!("Our reservation request accepted by relay:{relay_peer_id}");
            }
            SwarmEvent::Behaviour(BehaviourEvent::RelayClient(event)) => {
                console_log!("RelayClient: {event:?}");
            }
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(
                libp2p::gossipsub::Event::Message {
                    message_id: _,
                    propagation_source: _,
                    message,
                },
            )) => {
                console_log!(
                    "Received message from {:?}: {}",
                    message.source,
                    String::from_utf8(message.data).unwrap()
                );
            }
            SwarmEvent::Behaviour(BehaviourEvent::Gossipsub(
                libp2p::gossipsub::Event::Subscribed { peer_id, topic },
            )) => {
                console_log!("{peer_id} subscribed to {topic}");
            }
            SwarmEvent::Behaviour(BehaviourEvent::Identify(e)) => {
                console_log!("BehaviourEvent::Identify {e:?}");

                if let identify::Event::Error { peer_id, error } = e {
                    match error {
                        libp2p::swarm::StreamUpgradeError::Timeout => {
                            // When a browser tab closes, we don't get a swarm event
                            // maybe there's a way to get this with TransportEvent
                            // but for now remove the peer from routing table if there's an Identify timeout
                            swarm.behaviour_mut().kademlia.remove_peer(&peer_id);
                            console_log!(
                                "Removed {peer_id} from the routing table (if it was in there)."
                            );
                        }
                        _ => {
                            console_log!("StreamUpgradeError: {error}");
                        }
                    }
                } else if let identify::Event::Received {
                    peer_id,
                    info:
                        identify::Info {
                            listen_addrs,
                            protocols,
                            observed_addr,
                            ..
                        },
                } = e
                {
                    console_log!("identify::Event::Received observed_addr: {observed_addr}");

                    swarm.add_external_address(observed_addr, AddressScore::Infinite);

                    if protocols
                        .iter()
                        .any(|p| p.to_string() == KADEMLIA_PROTOCOL_NAME)
                    {
                        for addr in listen_addrs {
                            console_log!("identify::Event::Received listen addr: {addr}");
                            swarm
                                .behaviour_mut()
                                .kademlia
                                .add_address(&peer_id, addr.clone());

                            swarm
                                .behaviour_mut()
                                .kademlia
                                .add_address(&peer_id, addr.clone());
                            console_log!("Added {addr} to the routing table.");
                        }
                    }
                }
            }
            SwarmEvent::Behaviour(BehaviourEvent::Kademlia(e)) => {
                console_log!("Kademlia event: {e:?}");
            }
            event => {
                console_log!("Other type of event: {event:?}");
            }
        }
    }
}

fn create_swarm(
    local_key: identity::Keypair,
    local_peer_id: PeerId,
) -> Result<Swarm<Behaviour>, Box<dyn Error>> {
    // To content-address message, we can take the hash of message and use it as an ID.
    let message_id_fn = |message: &gossipsub::Message| {
        let mut s = DefaultHasher::new();
        message.data.hash(&mut s);
        gossipsub::MessageId::from(s.finish().to_string())
    };

    // Set a custom gossipsub configuration
    let gossipsub_config = gossipsub::ConfigBuilder::default()
        .validation_mode(gossipsub::ValidationMode::Permissive) // This sets the kind of message validation. The default is Strict (enforce message signing)
        .message_id_fn(message_id_fn) // content-address messages. No two messages of the same content will be propagated.
        .mesh_outbound_min(1)
        .mesh_n_low(1)
        .flood_publish(true)
        .build()
        .expect("Valid config");

    // build a gossipsub network behaviour
    let mut gossipsub = gossipsub::Behaviour::new(
        gossipsub::MessageAuthenticity::Signed(local_key.clone()),
        gossipsub_config,
    )
    .expect("Correct configuration");

    gossipsub.subscribe(&gossipsub::IdentTopic::new(GOSSIPSUB_TOPIC))?;

    let (relay_transport, relay_client) = relay::client::new(local_peer_id);

    let transport = {
        let ws_transport = ExtTransport::new(websocket_transport())
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::Config::new(&local_key).unwrap())
            .multiplex(yamux::Config::default())
            .timeout(Duration::from_secs(10))
            .boxed();

        let relay_transport = relay_transport
            .upgrade(upgrade::Version::V1)
            .authenticate(noise::Config::new(&local_key).unwrap())
            .multiplex(yamux::Config::default())
            .boxed();

        ws_transport
            .or_transport(relay_transport)
            .map(|fut, _| match fut {
                futures::future::Either::Right((local_peer_id, conn)) => {
                    (local_peer_id, StreamMuxerBox::new(conn))
                }
                futures::future::Either::Left((local_peer_id, conn)) => {
                    (local_peer_id, StreamMuxerBox::new(conn))
                }
            })
            .boxed()
    };

    let identify = identify::Behaviour::new(
        identify::Config::new("/ipfs/0.1.0".into(), local_key.public())
            .with_interval(Duration::from_secs(60)), // do this so we can get timeouts for dropped WebRTC connections
    );

    // Create a Kademlia behaviour.
    let mut cfg = KademliaConfig::default();
    cfg.set_protocol_names(vec![StreamProtocol::new(KADEMLIA_PROTOCOL_NAME)]);
    let kademlia = Kademlia::with_config(local_peer_id, MemoryStore::new(local_peer_id), cfg);

    let behaviour = Behaviour {
        gossipsub,
        identify,
        kademlia,
        keep_alive: keep_alive::Behaviour::default(),
        relay_client,
    };
    Ok(SwarmBuilder::with_wasm_executor(transport, behaviour, local_peer_id).build())
}

#[derive(NetworkBehaviour)]
struct Behaviour {
    gossipsub: gossipsub::Behaviour,
    identify: identify::Behaviour,
    kademlia: Kademlia<MemoryStore>,
    keep_alive: keep_alive::Behaviour,
    relay_client: relay::client::Behaviour,
}
