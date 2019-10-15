#[macro_use]
extern crate futures;
extern crate tokio;

use futures::future::{self, Either};
use futures::sync::mpsc;
use rand::Rng;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::codec::{Decoder, Framed};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;
use tokio::timer::Delay;
use log::*;

mod codec;

use self::codec::daemon::*;
use self::codec::peer::*;
use self::codec::server::*;

/// Shorthand for the transmit half of the message channel.
type Tx<T> = mpsc::UnboundedSender<T>;

/// Shorthand for the receive half of the message channel.
type Rx<T> = mpsc::UnboundedReceiver<T>;

// search peers apparently connect to the user after SConnectToPeer is sent; there is nothing the
// client does to initiate the connection themselves.
struct PendingPeer {
    username: String,
    use_obfuscation: bool,
}

struct State {
    server_tx: Option<Tx<ServerMsg>>,
    token: u32,

    /// clients connected to the museek interface
    clients: HashMap<u32, Tx<DaemonMsg>>,
}

impl State {
    pub fn new() -> Self {
        State {
            server_tx: None,
            token: 0,
            clients: HashMap::new(),
        }
    }

    pub fn next_token(&mut self) -> u32 {
        self.token += 1;
        self.token
    }

    pub fn send_server_message(
        &mut self,
        msg: ServerMsg,
    ) -> Result<(), mpsc::SendError<ServerMsg>> {
        self.server_tx.as_ref().unwrap().unbounded_send(msg)
    }

    pub fn send_daemon_message_all(
        &mut self,
        msg: DaemonMsg,
    ) -> Result<(), mpsc::SendError<DaemonMsg>> {
        for (_, tx) in self.clients.iter_mut() {
            tx.unbounded_send(msg.clone())?;
        }
        Ok(())
    }
}

type PeerSocket = Framed<TcpStream, PeerMsgCodec>;

struct SearchPeer {
    username: String,
    socket: PeerSocket,
    addr: SocketAddr,
    use_obfuscation: bool,
    state: Arc<Mutex<State>>,
}

impl Future for SearchPeer {
    type Item = ();
    type Error = std::io::Error;
    fn poll(&mut self) -> Poll<(), std::io::Error> {
        while let Ok(Async::Ready(msg_opt)) = self.socket.poll() {
            if let Some(msg) = msg_opt {
                debug!("RECV PEER msg {}: {:?}", self.username, msg);
                match msg {
                    PeerMsg::SearchReply {
                        username,
                        token,
                        results,
                        slots_free,
                        average_speed,
                        queue_length,
                        locked_results,
                    } => {
                        info!("Searhc: {:?}", results);
                        self.state.lock().unwrap().send_daemon_message_all(
                            DaemonMsg::SSearchReply {
                                token: token,
                                username: username,
                                slots_free: slots_free,
                                average_speed: average_speed,
                                // soulseek uses u64, but museek uses u32
                                queue_length: queue_length as u32,
                                results: results,
                                locked_results: locked_results.unwrap_or(Vec::new()),
                            },
                        );
                    }
                    _ => warn!("unimplemented peer msg: {:?}", msg),
                }
            }
        }

        try_ready!(self.socket.poll_complete());

        Ok(Async::NotReady)
    }
}

type ServerSocket = Framed<TcpStream, ServerMsgCodec>;

struct Server {
    state: Arc<Mutex<State>>,
    rx: Rx<ServerMsg>,
    socket: ServerSocket,

    logged_in: bool,
    username: String,
}

impl Server {
    pub fn new(mut socket: ServerSocket, state: Arc<Mutex<State>>, username: String) -> Self {
        let (tx, rx) = mpsc::unbounded();

        state.lock().unwrap().server_tx = Some(tx);

        socket
            .start_send(ServerMsg::CSetListenPort {
                port: 51672,
                use_obfuscation: false,
            })
            .unwrap();

        socket
            .start_send(ServerMsg::CSetListenPort {
                port: 51673,
                use_obfuscation: true,
            })
            .unwrap();

        socket
            .start_send(ServerMsg::CSharedFoldersFiles {
                folders: 1,
                files: 1,
            })
            .unwrap();

        socket
            .start_send(ServerMsg::CHaveNoParents { have_parents: true })
            .unwrap();

        socket
            .start_send(ServerMsg::CSetStatus {
                status: CSetStatusStatus::Online,
            })
            .unwrap();

        socket.start_send(
            ServerMsg::CFileSearch {
                token: 123,
                query: "squarepusher".into()
            }
        ).unwrap();

        Server {
            state: state,
            rx: rx,
            socket: socket,
            logged_in: false,
            username: username,
        }
    }

    fn connect_to_peer(
        &mut self,
        username: String,
        kind: SConnectToPeerKind,
        ip: Ipv4Addr,
        port: u32,
        token: u32,
        use_obfuscation: bool,
        privileged: bool,
        obfuscated_port: u32,
    ) {
        let port = if use_obfuscation {
            obfuscated_port
        } else {
            port
        };

        // let saddr = SocketAddrV4::new(ip, port as u16).into();

        match kind {
            SConnectToPeerKind::Peer => {
                let peer = PendingPeer {
                    username: username,
                    use_obfuscation: use_obfuscation,
                };
            }
            SConnectToPeerKind::Distributed => {}
            _ => (),
        }
    }

    fn login(&mut self, success: bool) {
        self.logged_in = success;
        self.on_server_state_change(success);
    }

    fn connect_to_parent_peers(&mut self, users: HashMap<String, (Ipv4Addr, u32)>) {
        for (user, (ip, port)) in users {
            let token = self.state.lock().unwrap().next_token();

            self.socket
                .start_send(ServerMsg::CParentIp { ip: ip })
                .unwrap();

            self.connect_to_peer(
                user,
                SConnectToPeerKind::Distributed,
                ip,
                port,
                token,
                false,
                false,
                0,
            );
        }
    }

    fn on_server_state_change(&mut self, success: bool) {
        self.state
            .lock()
            .unwrap()
            .send_daemon_message_all(DaemonMsg::SServerState {
                connected: success,
                username: self.username.clone(),
            })
            .unwrap();
    }
}

enum ServerResult {
    ServerDisconnected,
    UserDisconnected,
}

impl Future for Server {
    type Item = ServerResult;
    type Error = std::io::Error;
    fn poll(&mut self) -> Poll<ServerResult, std::io::Error> {
        while let Ok(Async::Ready(msg_opt)) = self.socket.poll() {
            if let Some(msg) = msg_opt {
                debug!("RECV SERVER msg: {:?}", msg);
                match msg {
                    ServerMsg::SLogin {
                        success,
                        greet,
                        public_ip,
                        unknown,
                    } => {
                        self.login(success);
                    }
                    ServerMsg::SConnectToPeer {
                        username,
                        kind,
                        ip,
                        port,
                        token,
                        use_obfuscation,
                        privileged,
                        obfuscated_port,
                    } => {
                        self.connect_to_peer(
                            username,
                            kind,
                            ip,
                            port,
                            token,
                            use_obfuscation,
                            privileged,
                            obfuscated_port,
                        );
                    }
                    ServerMsg::SNetInfo { users } => {
                        self.connect_to_parent_peers(users);
                    }
                    _ => (),
                }
            } else {
                self.on_server_state_change(false);
                return Ok(Async::Ready(ServerResult::ServerDisconnected));
            }
        }

        // daemon messages
        for i in 0..10 {
            match self.rx.poll().unwrap() {
                Async::Ready(Some(msg)) => {
                    info!("From daemon: {:?}", msg);
                    self.socket.start_send(msg)?;

                    if i + 1 == 10 {
                        task::current().notify();
                    }
                }
                _ => break,
            }
        }

        // flush messages
        try_ready!(self.socket.poll_complete());

        Ok(Async::NotReady)
    }
}

type DaemonSocket = Framed<TcpStream, DaemonMsgCodec>;

struct DaemonClient {
    state: Arc<Mutex<State>>,
    socket: DaemonSocket,
    token: u32,
    rx: Rx<DaemonMsg>,
}

impl DaemonClient {
    pub fn new(socket: DaemonSocket, state: Arc<Mutex<State>>) -> Self {
        let token = state.lock().unwrap().next_token();

        let (tx, rx) = mpsc::unbounded();
        state.lock().unwrap().clients.insert(token, tx);

        // hack
        socket.get_ref().set_recv_buffer_size(1000000).unwrap();
        socket.get_ref().set_send_buffer_size(1000000).unwrap();

        DaemonClient {
            state: state,
            socket: socket,
            token: token,
            rx: rx,
        }
    }

    fn login(
        &mut self,
        algorithm: codec::daemon::CLoginAlgorithm,
        challenge_response: String,
        mask: u32,
    ) {
        info!("login client {:?}", challenge_response);

        self.socket
            .start_send(DaemonMsg::SLogin {
                success: true,
                message: String::new(),
                challenge: String::new(),
            })
            .unwrap();
    }

    fn search(&mut self, kind: codec::daemon::CSearchKind, query: &str) {
        info!("search {:?} {:?}", kind, query);

        let token = self.state.lock().unwrap().next_token();

        match kind {
            codec::daemon::CSearchKind::Global => {
                self.state.lock().unwrap().send_server_message({
                    ServerMsg::CFileSearch {
                        token: token,
                        query: query.into(),
                    }
                });
                self.state.lock().unwrap().send_server_message({
                    ServerMsg::CRelatedSearch {
                        query: query.into(),
                    }
                });
            }
            _ => (),
        }

        self.socket
            .start_send(DaemonMsg::SSearch {
                query: query.into(),
                token: token,
            })
            .unwrap();
    }
}

impl Future for DaemonClient {
    type Item = ();
    type Error = std::io::Error;
    fn poll(&mut self) -> Poll<(), std::io::Error> {
        // TODO limit
        loop {
            match self.socket.poll() {
                Ok(Async::Ready(Some(msg))) => match msg {
                    DaemonMsg::CLogin {
                        algorithm,
                        challenge_response,
                        mask,
                    } => self.login(algorithm, challenge_response, mask),
                    DaemonMsg::CSearch { kind, query } => self.search(kind, &query),
                    _ => {
                        warn!("Unimplemented daemon msg: {:?}", msg);
                    }
                },
                Ok(Async::Ready(None)) => {
                    info!("client disconnect");
                    return Ok(Async::Ready(()));
                }
                Ok(Async::NotReady) => break,
                Err(e) => {
                    error!("client err: {:?}", e);
                    return Ok(Async::Ready(()));
                }
            }
        }

        // messages from elsewhere
        for i in 0..10 {
            match self.rx.poll().unwrap() {
                Async::Ready(Some(msg)) => {
                    info!("From elsewhere: {:?}", msg);
                    self.socket.start_send(msg)?;

                    if i + 1 == 10 {
                        task::current().notify();
                    }
                }
                _ => break,
            }
        }

        // flush messages
        try_ready!(self.socket.poll_complete());

        Ok(Async::NotReady)
    }
}

fn server_task(state: Arc<Mutex<State>>) -> impl Future<Item = ServerResult, Error = ()> {
    let addr = "server.slsknet.org:2242";

    let saddr = addr.to_socket_addrs().unwrap().next().unwrap();
    TcpStream::connect(&saddr)
        .and_then(move |socket| {
            info!("connected");

            let username = String::from("akeshi");
            let password = String::from("bakamitai");

            let framed = ServerMsgCodec::new().framed(socket);
            framed
                .send(ServerMsg::CLogin {
                    username: username.clone(),
                    password: password,
                })
                .and_then(move |socket| {
                    info!("trying to connect to {}", &saddr);
                    Server::new(socket, state.clone(), username)
                })
        })
        .map_err(|e| error!("server error: {:?}", e))
}

fn listener_task(state: Arc<Mutex<State>>, port: u16, use_obfuscation: bool) -> impl Future<Item = (), Error = ()> {
    let saddr = SocketAddrV4::new(Ipv4Addr::new(192, 168, 1, 101), port).into();

    TcpListener::bind(&saddr)
        .expect("failed to bind listener")
        .incoming()
        .for_each(move |socket| {
            let saddr = socket.peer_addr().unwrap();
            info!("incoming peer: {:?}", saddr);
            let framed = PeerMsgCodec::new(use_obfuscation).framed(socket);
            let peer = SearchPeer {
                username: "asd".into(),
                socket: framed,
                addr: saddr,
                use_obfuscation: use_obfuscation,
                state: state.clone(),
            }
            .map_err(|e| {
                error!("listener error: {:?}", e);
            });
            tokio::spawn(peer);
            Ok(())
        })
        .map_err(|e| {
            error!("listener error: {:?}", e);
        })
}

static CHALLENGE_MAP: &'static str = "0123456789abcdef";

fn challenge() -> String {
    let mut result = String::new();
    let mut rng = rand::thread_rng();

    for _ in 0..64 {
        result.push(CHALLENGE_MAP.chars().nth(rng.gen::<usize>() % 16).unwrap())
    }

    result
}

fn process_daemon_socket(socket: TcpStream, state: Arc<Mutex<State>>) {
    info!("client connecting");

    let codec = DaemonMsgCodec::new().framed(socket);

    let connection = codec
        .send(DaemonMsg::SChallenge {
            version: 4,
            challenge: challenge(),
        })
        .and_then(move |socket| DaemonClient::new(socket, state.clone()))
        .map_err(|_| ());

    tokio::spawn(connection);
}

fn daemon_task(state: Arc<Mutex<State>>) -> impl Future<Item = (), Error = ()> {
    let addr = "127.0.0.1:12345";
    let saddr = addr.to_socket_addrs().unwrap().next().unwrap();

    TcpListener::bind(&saddr)
        .expect("failed to bind daemon")
        .incoming()
        .for_each(move |socket| {
            process_daemon_socket(socket, state.clone());
            Ok(())
        })
        .map_err(|e| {
            error!("daemon error: {:?}", e);
        })
}

fn main() {
    env_logger::init();

    let state = Arc::new(Mutex::new(State::new()));

    let server = server_task(state.clone());
    let daemon = daemon_task(state.clone());
    let listener = listener_task(state.clone(), 51672, false);
    let obfs_listener = listener_task(state.clone(), 51673, true);

    let reconn = server.and_then(|result| {
        match result {
            ServerResult::ServerDisconnected => {
                // try to reconnect
                info!("server disconnected, trying to reconnect");
                let reconnect_delay = 2000;
                let when = Instant::now() + Duration::from_millis(reconnect_delay);
                let cls = Delay::new(when)
                    .map_err(|_| ())
                    .and_then(move |_| server_task(state.clone()).map(|_| ()));
                Either::A(cls)
            }
            ServerResult::UserDisconnected => {
                // do nothing
                Either::B(future::ok(()))
            }
        }
    });

    let task = reconn.join(daemon).join(listener).join(obfs_listener).map_err(|_| ()).map(|_| ());
    tokio::run(task);
}
