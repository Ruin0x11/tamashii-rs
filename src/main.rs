#[macro_use]
extern crate futures;
extern crate tokio;

use futures::future::{self, Either};
use futures::sync::mpsc;
use rand::Rng;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::codec::{Decoder, Framed};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;
use tokio::timer::Delay;

mod codec;

use self::codec::daemon::*;
use self::codec::peer::*;
use self::codec::server::*;

/// Shorthand for the transmit half of the message channel.
type Tx = mpsc::UnboundedSender<ServerMsg>;

/// Shorthand for the receive half of the message channel.
type Rx = mpsc::UnboundedReceiver<ServerMsg>;

struct State {
    server_tx: Option<Tx>,
    token: u32,
    logged_in: bool,
}

impl State {
    pub fn new() -> Self {
        State {
            server_tx: None,
            token: 0,
            logged_in: false,
        }
    }

    pub fn next_token(&mut self) -> u32 {
        self.token += 1;
        self.token
    }

    pub fn send_server_message(&mut self, msg: ServerMsg) {
        self.server_tx.as_ref().unwrap().unbounded_send(msg);
    }
}

type PeerSocket = Framed<TcpStream, PeerMsgCodec>;

struct SearchPeer {
    username: String,
    socket: PeerSocket,
    addr: SocketAddr,
    use_obfuscation: bool,
}

impl Future for SearchPeer {
    type Item = ();
    type Error = std::io::Error;
    fn poll(&mut self) -> Poll<(), std::io::Error> {
        while let Ok(Async::Ready(msg_opt)) = self.socket.poll() {
            if let Some(msg) = msg_opt {
                println!("RECV PEER msg {}: {:?}", self.username, msg);
            } else {
                println!("not ready");
            }
        }

        try_ready!(self.socket.poll_complete());

        Ok(Async::NotReady)
    }
}

type ServerSocket = Framed<TcpStream, ServerMsgCodec>;

struct Server {
    state: Arc<Mutex<State>>,
    rx: Rx,
    socket: ServerSocket,
}

impl Server {
    pub fn new(mut socket: ServerSocket, state: Arc<Mutex<State>>) -> Self {
        let (tx, rx) = mpsc::unbounded();

        state.lock().unwrap().server_tx = Some(tx);

        socket
            .start_send(ServerMsg::CSetListenPort {
                port: 2234,
                use_obfuscation: false,
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

        Server {
            state: state,
            rx: rx,
            socket: socket,
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

        let saddr = SocketAddrV4::new(ip, port as u16).into();
        let state = self.state.clone();

        println!("peer token {} {:#08x} {}", username, token, saddr);

        match kind {
            SConnectToPeerKind::Peer => {
                let connection = TcpStream::connect(&saddr)
                    .and_then(move |socket| {
                        let framed = PeerMsgCodec::new().framed(socket);
                        framed
                            .send(PeerMsg::HPierceFirewall { token: token })
                            .and_then(move |socket| {
                                println!("Connected to peer {} {}", username, saddr);
                                SearchPeer {
                                    username: username,
                                    socket: socket,
                                    addr: saddr,
                                    use_obfuscation: use_obfuscation,
                                }
                            })
                    })
                    .map_err(|e| eprintln!("peer connect error: {:?}", e));

                tokio::spawn(connection);
            }
            SConnectToPeerKind::Distributed => {}
            _ => (),
        }
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
                println!("RECV SERVER msg: {:?}", msg);
                match msg {
                    ServerMsg::SLogin {
                        success,
                        greet,
                        public_ip,
                        unknown,
                    } => {
                        self.state.lock().unwrap().logged_in = success;
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
                        for (user, (ip, port)) in users {
                            println!("parent {}", ip);

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
                    _ => (),
                }
            } else {
                return Ok(Async::Ready(ServerResult::ServerDisconnected));
            }
        }

        // daemon messages
        for i in 0..10 {
            match self.rx.poll().unwrap() {
                Async::Ready(Some(msg)) => {
                    println!("From daemon: {:?}", msg);
                    self.socket.start_send(msg);

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
}

impl DaemonClient {
    pub fn new(socket: DaemonSocket, state: Arc<Mutex<State>>) -> Self {
        DaemonClient {
            state: state,
            socket: socket,
        }
    }

    fn login(
        &mut self,
        algorithm: codec::daemon::CLoginAlgorithm,
        challenge_response: String,
        mask: u32,
    ) {
        println!("login client {:?}", challenge_response);

        self.socket
            .start_send(DaemonMsg::SLogin {
                success: true,
                message: String::new(),
                challenge: String::new(),
            })
            .unwrap();
    }

    fn search(&mut self, kind: codec::daemon::CSearchKind, query: &str) {
        println!("search {:?} {:?}", kind, query);

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
        while let Ok(Async::Ready(msg_opt)) = self.socket.poll() {
            if let Some(msg) = msg_opt {
                match msg {
                    DaemonMsg::CLogin {
                        algorithm,
                        challenge_response,
                        mask,
                    } => self.login(algorithm, challenge_response, mask),
                    DaemonMsg::CSearch { kind, query } => self.search(kind, &query),
                    _ => {
                        eprintln!("Unimplemented daemon msg: {:?}", msg);
                    }
                }
            } else {
                println!("client disconnect");
                return Ok(Async::Ready(()));
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
            println!("connected");

            let framed = ServerMsgCodec::new().framed(socket);
            framed
                .send(ServerMsg::CLogin {
                    username: "akeshi".into(),
                    password: "bakamitai".into(),
                })
                .and_then(move |socket| {
                    println!("trying to connect to {}", &saddr);
                    Server::new(socket, state.clone())
                })
        })
        .map_err(|e| eprintln!("server error: {:?}", e))
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
            eprintln!("daemon error: {:?}", e);
        })
}

fn main() {
    let state = Arc::new(Mutex::new(State::new()));

    let server = server_task(state.clone());
    let daemon = daemon_task(state.clone());

    let reconn = server.and_then(|result| {
        match result {
            ServerResult::ServerDisconnected => {
                // try to reconnect
                println!("server disconnected, trying to reconnect");
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

    let task = reconn.join(daemon).map_err(|_| ()).map(|_| ());
    tokio::run(task);
}
