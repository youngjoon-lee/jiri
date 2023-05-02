use libp2p::core::{muxing::StreamMuxerBox, transport::Boxed};
use libp2p::identity::Keypair;
use libp2p::PeerId;
use std::error::Error;

#[cfg(not(feature = "web"))]
pub fn create_transport(
    id_keys: &Keypair,
) -> Result<Boxed<(PeerId, StreamMuxerBox)>, Box<dyn Error>> {
    use futures::future::Either;
    use libp2p::{
        core::{transport::OrTransport, upgrade::Version},
        noise, tcp, websocket, yamux, Transport,
    };

    let tcp_transport = tcp::tokio::Transport::default()
        .upgrade(Version::V1Lazy)
        .authenticate(noise::NoiseAuthenticated::xx(&id_keys)?)
        .multiplex(yamux::YamuxConfig::default())
        .boxed();
    let ws_transport = websocket::WsConfig::new(tcp::tokio::Transport::new(tcp::Config::new()))
        .upgrade(Version::V1)
        .authenticate(noise::NoiseAuthenticated::xx(&id_keys)?)
        .multiplex(yamux::YamuxConfig::default())
        .boxed();
    let transport = OrTransport::new(tcp_transport, ws_transport)
        .map(|either_output, _| match either_output {
            Either::Left((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
            Either::Right((peer_id, muxer)) => (peer_id, StreamMuxerBox::new(muxer)),
        })
        .boxed();

    Ok(transport)
}

#[cfg(feature = "web")]
pub fn create_transport() -> Result<Boxed<(PeerId, StreamMuxerBox)>, Box<dyn Error>> {
    Err("dummy") //TODO: implement
}
