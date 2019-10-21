// Copyright 2019 Parity Technologies (UK) Ltd.
//
// Permission is hereby granted, free of charge, to any person obtaining a
// copy of this software and associated documentation files (the "Software"),
// to deal in the Software without restriction, including without limitation
// the rights to use, copy, modify, merge, publish, distribute, sublicense,
// and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING
// FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER
// DEALINGS IN THE SOFTWARE.

mod util;

use futures::prelude::*;
use libp2p_core::{identity, upgrade, Transport};
use libp2p_core::nodes::{Network, NetworkEvent, Peer};
use libp2p_core::nodes::network::IncomingError;
use libp2p_swarm::{
    ProtocolsHandler,
    KeepAlive,
    SubstreamProtocol,
    ProtocolsHandlerEvent,
    ProtocolsHandlerUpgrErr,
};
use std::{io, pin::Pin, task::Context, task::Poll, time::Duration};
use wasm_timer::Delay;

// TODO: replace with DummyProtocolsHandler after https://github.com/servo/rust-smallvec/issues/139 ?
struct TestHandler<TSubstream>(std::marker::PhantomData<TSubstream>);

impl<TSubstream> Default for TestHandler<TSubstream> {
    fn default() -> Self {
        TestHandler(std::marker::PhantomData)
    }
}

impl<TSubstream> ProtocolsHandler for TestHandler<TSubstream>
where
    TSubstream: AsyncRead + AsyncWrite + Unpin + Send + 'static
{
    type InEvent = ();      // TODO: cannot be Void (https://github.com/servo/rust-smallvec/issues/139)
    type OutEvent = ();      // TODO: cannot be Void (https://github.com/servo/rust-smallvec/issues/139)
    type Error = io::Error;
    type Substream = TSubstream;
    type InboundProtocol = upgrade::DeniedUpgrade;
    type OutboundProtocol = upgrade::DeniedUpgrade;
    type OutboundOpenInfo = ();      // TODO: cannot be Void (https://github.com/servo/rust-smallvec/issues/139)

    fn listen_protocol(&self) -> SubstreamProtocol<Self::InboundProtocol> {
        SubstreamProtocol::new(upgrade::DeniedUpgrade)
    }

    fn inject_fully_negotiated_inbound(
        &mut self,
        _: <Self::InboundProtocol as upgrade::InboundUpgrade<Self::Substream>>::Output
    ) { panic!() }

    fn inject_fully_negotiated_outbound(
        &mut self,
        _: <Self::OutboundProtocol as upgrade::OutboundUpgrade<Self::Substream>>::Output,
        _: Self::OutboundOpenInfo
    ) { panic!() }

    fn inject_event(&mut self, _: Self::InEvent) {
        panic!()
    }

    fn inject_dial_upgrade_error(&mut self, _: Self::OutboundOpenInfo, _: ProtocolsHandlerUpgrErr<<Self::OutboundProtocol as upgrade::OutboundUpgrade<Self::Substream>>::Error>) {

    }

    fn connection_keep_alive(&self) -> KeepAlive { KeepAlive::Yes }

    fn poll(&mut self, _: &mut Context) -> Poll<ProtocolsHandlerEvent<Self::OutboundProtocol, Self::OutboundOpenInfo, Self::OutEvent, Self::Error>> {
        Poll::Pending
    }
}

#[test]
fn raw_swarm_simultaneous_connect() {
    // Checks whether two swarms dialing each other simultaneously properly works.

    // When two swarms A and B dial each other, the following can happen:
    //
    // - A and B both successfully open a dialing connection simultaneously, then either A or B
    //   (but not both) closes its dialing connection and get a `Replaced` event to replace the
    //   dialing connection with the listening one. The other one gets an `IncomingConnectionError`.
    // - A successfully dials B; B doesn't have dialing priority and thus cancels its dialing
    //   attempt. If A receives B's dialing attempt, it gets an `IncomingConnectionError`.
    // - A successfully dials B; B does have dialing priority and thus continues dialing; then B
    //   successfully dials A; A and B both get a `Replaced` event to replace the dialing
    //   connection with the listening one.
    //

    // Important note: This test is meant to detect race conditions which don't seem to happen
    //                 if we use the `MemoryTransport`. Using the TCP transport is important,
    //                 despite the fact that it adds a dependency.

    for _ in 0 .. 10 {
        let mut swarm1 = {
            let local_key = identity::Keypair::generate_ed25519();
            let local_public_key = local_key.public();
            let transport = libp2p_tcp::TcpConfig::new()
                .upgrade(upgrade::Version::V1Lazy)
                .authenticate(libp2p_secio::SecioConfig::new(local_key))
                .multiplex(libp2p_mplex::MplexConfig::new())
                .and_then(|(peer, mplex), _| {
                    // Gracefully close the connection to allow protocol
                    // negotiation to complete.
                    util::CloseMuxer::new(mplex).map_ok(move |mplex| (peer, mplex))
                });
            Network::new(transport, local_public_key.into_peer_id())
        };

        let mut swarm2 = {
            let local_key = identity::Keypair::generate_ed25519();
            let local_public_key = local_key.public();
            let transport = libp2p_tcp::TcpConfig::new()
                .upgrade(upgrade::Version::V1Lazy)
                .authenticate(libp2p_secio::SecioConfig::new(local_key))
                .multiplex(libp2p_mplex::MplexConfig::new())
                .and_then(|(peer, mplex), _| {
                    // Gracefully close the connection to allow protocol
                    // negotiation to complete.
                    util::CloseMuxer::new(mplex).map_ok(move |mplex| (peer, mplex))
                });
            Network::new(transport, local_public_key.into_peer_id())
        };

        swarm1.listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap()).unwrap();
        swarm2.listen_on("/ip4/127.0.0.1/tcp/0".parse().unwrap()).unwrap();

        let (swarm1_listen_addr, swarm2_listen_addr, mut swarm1, mut swarm2) = futures::executor::block_on(
            future::lazy(move |cx| {
                let swarm1_listen_addr =
                    if let Poll::Ready(NetworkEvent::NewListenerAddress { listen_addr, .. }) = swarm1.poll(cx) {
                        listen_addr
                    } else {
                        panic!("Was expecting the listen address to be reported")
                    };

                let swarm2_listen_addr =
                    if let Poll::Ready(NetworkEvent::NewListenerAddress { listen_addr, .. }) = swarm2.poll(cx) {
                        listen_addr
                    } else {
                        panic!("Was expecting the listen address to be reported")
                    };

                Ok::<_, void::Void>((swarm1_listen_addr, swarm2_listen_addr, swarm1, swarm2))
            })
        ).unwrap();

        loop {
            let mut swarm1_step = 0;
            let mut swarm2_step = 0;

            let mut swarm1_dial_start = Delay::new(Duration::new(0, rand::random::<u32>() % 50_000_000));
            let mut swarm2_dial_start = Delay::new(Duration::new(0, rand::random::<u32>() % 50_000_000));

            let future = future::poll_fn(|cx| -> Poll<bool> {
                loop {
                    let mut swarm1_not_ready = false;
                    let mut swarm2_not_ready = false;

                    // We add a lot of randomness. In a real-life situation the swarm also has to
                    // handle other nodes, which may delay the processing.

                    if swarm1_step == 0 {
                        match Future::poll(Pin::new(&mut swarm1_dial_start), cx) {
                            Poll::Ready(_) => {
                                let handler = TestHandler::default().into_node_handler_builder();
                                swarm1.peer(swarm2.local_peer_id().clone())
                                    .into_not_connected()
                                    .unwrap()
                                    .connect(swarm2_listen_addr.clone(), handler);
                                swarm1_step = 1;
                            },
                            Poll::Pending => swarm1_not_ready = true,
                        }
                    }

                    if swarm2_step == 0 {
                        match Future::poll(Pin::new(&mut swarm2_dial_start), cx) {
                            Poll::Ready(_) => {
                                let handler = TestHandler::default().into_node_handler_builder();
                                swarm2.peer(swarm1.local_peer_id().clone())
                                    .into_not_connected()
                                    .unwrap()
                                    .connect(swarm1_listen_addr.clone(), handler);
                                swarm2_step = 1;
                            },
                            Poll::Pending => swarm2_not_ready = true,
                        }
                    }

                    if rand::random::<f32>() < 0.1 {
                        match swarm1.poll(cx) {
                            Poll::Ready(NetworkEvent::IncomingConnectionError {
                                error: IncomingError::DeniedLowerPriority, ..
                            }) => {
                                assert_eq!(swarm1_step, 2);
                                swarm1_step = 3;
                            },
                            Poll::Ready(NetworkEvent::Connected { conn_info, .. }) => {
                                assert_eq!(conn_info, *swarm2.local_peer_id());
                                if swarm1_step == 0 {
                                    // The connection was established before
                                    // swarm1 started dialing; discard the test run.
                                    return Poll::Ready(false)
                                }
                                assert_eq!(swarm1_step, 1);
                                swarm1_step = 2;
                            },
                            Poll::Ready(NetworkEvent::Replaced { new_info, .. }) => {
                                assert_eq!(new_info, *swarm2.local_peer_id());
                                assert_eq!(swarm1_step, 2);
                                swarm1_step = 3;
                            },
                            Poll::Ready(NetworkEvent::IncomingConnection(inc)) => {
                                inc.accept(TestHandler::default().into_node_handler_builder());
                            },
                            Poll::Ready(ev) => panic!("swarm1: unexpected event: {:?}", ev),
                            Poll::Pending => swarm1_not_ready = true,
                        }
                    }

                    if rand::random::<f32>() < 0.1 {
                        match swarm2.poll(cx) {
                            Poll::Ready(NetworkEvent::IncomingConnectionError {
                                error: IncomingError::DeniedLowerPriority, ..
                            }) => {
                                assert_eq!(swarm2_step, 2);
                                swarm2_step = 3;
                            },
                            Poll::Ready(NetworkEvent::Connected { conn_info, .. }) => {
                                assert_eq!(conn_info, *swarm1.local_peer_id());
                                if swarm2_step == 0 {
                                    // The connection was established before
                                    // swarm2 started dialing; discard the test run.
                                    return Poll::Ready(false)
                                }
                                assert_eq!(swarm2_step, 1);
                                swarm2_step = 2;
                            },
                            Poll::Ready(NetworkEvent::Replaced { new_info, .. }) => {
                                assert_eq!(new_info, *swarm1.local_peer_id());
                                assert_eq!(swarm2_step, 2);
                                swarm2_step = 3;
                            },
                            Poll::Ready(NetworkEvent::IncomingConnection(inc)) => {
                                inc.accept(TestHandler::default().into_node_handler_builder());
                            },
                            Poll::Ready(ev) => panic!("swarm2: unexpected event: {:?}", ev),
                            Poll::Pending => swarm2_not_ready = true,
                        }
                    }

                    // TODO: make sure that >= 5 is correct
                    if swarm1_step + swarm2_step >= 5 {
                        return Poll::Ready(true);
                    }

                    if swarm1_not_ready && swarm2_not_ready {
                        return Poll::Pending;
                    }
                }
            });

            if futures::executor::block_on(future) {
                // The test exercised what we wanted to exercise: a simultaneous connect.
                break
            } else {
                // The test did not trigger a simultaneous connect; ensure the nodes
                // are disconnected and re-run the test.
                match swarm1.peer(swarm2.local_peer_id().clone()) {
                    Peer::Connected(p) => p.close(),
                    Peer::PendingConnect(p) => p.interrupt(),
                    x => panic!("Unexpected state for swarm1: {:?}", x)
                }
                match swarm2.peer(swarm1.local_peer_id().clone()) {
                    Peer::Connected(p) => p.close(),
                    Peer::PendingConnect(p) => p.interrupt(),
                    x => panic!("Unexpected state for swarm2: {:?}", x)
                }
            }
        }
    }
}

