#[macro_use]
extern crate futures;

use tokio::io;
use tokio::net::{TcpListener, TcpStream};
use tokio::prelude::*;
use tokio::timer::Interval;

struct Dood {}

impl Dood {
    pub fn new() -> Self {
        Dood {}
    }
}

impl Future for Dood {
    type Item = String;
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        Ok(Async::Ready("dood".to_string()))
    }
}

struct Adder {
    sum: i32,
    amount: i32,
    times: u32,
    now: u32,
    interval: Interval,
}

impl Adder {
    pub fn new(amount: i32, times: u32) -> Self {
        Adder {
            sum: 0,
            times: times,
            amount: amount,
            now: 0,
            interval: Interval::new_interval(std::time::Duration::from_secs(1)),
        }
    }
}

impl Stream for Adder {
    type Item = i32;
    type Error = ();

    fn poll(&mut self) -> Poll<Option<Self::Item>, Self::Error> {
        if self.now > self.times {
            return Ok(Async::Ready(None));
        }

        try_ready!(self.interval.poll().map_err(|_| ()));

        self.sum += self.amount;
        self.now += 1;

        Ok(Async::Ready(Some(self.sum)))
    }
}

struct Display<T>(T);

impl<T> Future for Display<T>
where
    T: Future,
    T::Item: std::fmt::Display,
{
    type Item = ();
    type Error = T::Error;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        let value = try_ready!(self.0.poll());

        println!("{}", value);

        Ok(().into())
    }
}

struct GetPeerAddr {
    future: tokio::net::ConnectFuture,
}

impl GetPeerAddr {
    pub fn new(addr: &std::net::SocketAddr) -> Self {
        GetPeerAddr {
            future: TcpStream::connect(addr),
        }
    }
}

impl Future for GetPeerAddr {
    type Item = ();
    type Error = ();

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.future.poll() {
            Ok(Async::Ready(s)) => {
                println!("socket: {:?}", s.peer_addr().unwrap());
                Ok(Async::Ready(()))
            }
            Ok(Async::NotReady) => Ok(Async::NotReady),
            Err(e) => {
                eprintln!("error: {}", e);
                Ok(Async::Ready(()))
            }
        }
    }
}

fn main() {
    let addr = "127.0.0.1:12345".parse().unwrap();
    // let client = TcpStream::connect(&addr)
    //     .and_then(|stream| {
    //         println!("connected to server");

    //         io::write_all(stream, "dood").then(|res| {
    //             println!("wrote stream, success {:?}", res.is_ok());
    //             Ok(())
    //         })
    //     })
    //     .map_err(|err| println!("connection error: {:?}", err));

    println!("listening on {:?}", addr);

    let listener = TcpListener::bind(&addr).expect("failed to bind");
    let future = listener
        .incoming()
        .for_each(|socket| {
            let (r, w) = socket.split();
            let amount = io::copy(r, w);
            let msg = amount.then(|result| {
                match result {
                    Ok((amount, _, _)) => {
                        let dood = Dood::new();
                        let disp = Display(dood);
                        tokio::spawn(disp);
                        tokio::spawn(Adder::new(10, 11).map_err(|_| ()).for_each(|n| {
                            println!("{}", n);
                            Ok(())
                        }));
                        tokio::spawn(Adder::new(1, 13).map_err(|_| ()).for_each(|n| {
                            println!("{}", n);
                            Ok(())
                        }));
                        tokio::spawn(GetPeerAddr::new(&"127.0.0.1:12346".parse().unwrap()));
                        println!("wrote {} bytes", amount)
                    }
                    Err(e) => eprintln!("error: {:?}", e),
                }

                Ok(())
            });

            tokio::spawn(msg);

            Ok(())
        })
        .map_err(|e| eprintln!("failed to accept socket, error {:?}", e));

    tokio::run(future);
}
