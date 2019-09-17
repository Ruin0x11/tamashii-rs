#[macro_use] extern crate futures;
#[macro_use] extern crate tokio;

use std::net::ToSocketAddrs;
use tokio::codec::{Decoder, Encoder, Framed};
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;

mod codec;

use self::codec::server::*;

type Data = i32;

type ServerSocket = Framed<TcpStream, ServerMsgCodec>;

struct Server {
    socket: ServerSocket,
}

impl Server {
    pub fn new(socket: ServerSocket) -> Self {
        Server { socket: socket }
    }
}

impl Future for Server {
    type Item = ();
    type Error = std::io::Error;
    fn poll(&mut self) -> Poll<(), std::io::Error> {
        while let Ok(Async::Ready(msg_opt)) = self.socket.poll() {
            if let Some(msg) = msg_opt {
                println!("msg {:?}", msg);
            }
        }

        Ok(Async::NotReady)
    }
}

fn server_task() -> impl Future<Item = (), Error = ()> {
    let addr = "server.slsknet.org:2242";

    let saddr = addr.to_socket_addrs().unwrap().next().unwrap();
    TcpStream::connect(&saddr)
        .and_then(|socket| {
            println!("connected");

            let framed = ServerMsgCodec::new().framed(socket);
            tokio::spawn(
                framed
                    .send(ServerMsg::CLogin {
                        username: "nonbirithm".into(),
                        password: "password".into(),
                    })
                    .map_err(|e| {
                        eprintln!("err = {:?}", e);
                        e
                    })
                    .and_then(Server::new)
                    .map_err(|e| eprintln!("err = {:?}", e)),
            );

            Ok(())
        })
        .map_err(|_| ())
}

fn main() {
    let cls = server_task();
    tokio::run(cls);
}
