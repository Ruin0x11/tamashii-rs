#[macro_use]
extern crate futures;
extern crate tokio;

use bytes::Bytes;
use futures::sync::mpsc;
use rand::Rng;
use std::net::ToSocketAddrs;
use std::sync::{Arc, Mutex};
use tokio::codec::{Decoder, Framed};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

mod codec;

use self::codec::daemon::{DaemonMsg, DaemonMsgCodec};
use self::codec::server::{ServerMsg, ServerMsgCodec};

/// Shorthand for the transmit half of the message channel.
type Tx = mpsc::UnboundedSender<ServerMsg>;

/// Shorthand for the receive half of the message channel.
type Rx = mpsc::UnboundedReceiver<ServerMsg>;

struct State {
    server_tx: Option<Tx>,
    token: u32,
}

impl State {
    pub fn new() -> Self {
        State {
            server_tx: None,
            token: 0,
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

type ServerSocket = Framed<TcpStream, ServerMsgCodec>;

struct Server {
    state: Arc<Mutex<State>>,
    rx: Rx,
    socket: ServerSocket,
}

impl Server {
    pub fn new(socket: ServerSocket, state: Arc<Mutex<State>>) -> Self {
        let (tx, rx) = mpsc::unbounded();

        state.lock().unwrap().server_tx = Some(tx);

        Server {
            state: state,
            rx: rx,
            socket: socket,
        }
    }
}

impl Future for Server {
    type Item = ();
    type Error = std::io::Error;
    fn poll(&mut self) -> Poll<(), std::io::Error> {
        while let Ok(Async::Ready(msg_opt)) = self.socket.poll() {
            if let Some(msg) = msg_opt {
                println!("msg {:?}", msg);
            } else {
                println!("server disconnect");
                return Ok(Async::Ready(()));
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
                },
                _ => break
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

fn server_task(state: Arc<Mutex<State>>) -> impl Future<Item = (), Error = ()> {
    let addr = "server.slsknet.org:2242";

    let saddr = addr.to_socket_addrs().unwrap().next().unwrap();
    TcpStream::connect(&saddr)
        .and_then(move |socket| {
            println!("connected");

            let framed = ServerMsgCodec::new().framed(socket);
            framed
                .send(ServerMsg::CLogin {
                    username: "akeshi".into(),
                    password: "password".into(),
                })
                .and_then(move |socket| Server::new(socket, state.clone()))
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
        .map_err(|e| eprintln!("daemon error: {:?}", e))
}

fn main() {
    let state = Arc::new(Mutex::new(State::new()));

    let server = server_task(state.clone());
    let daemon = daemon_task(state.clone());

    let task = server.join(daemon).map_err(|_| ()).map(|_| ());
    tokio::run(task);
}
