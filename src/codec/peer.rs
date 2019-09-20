use super::util;
use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::result::Result;
use tokio::codec::{Decoder, Encoder};

#[derive(Debug)]
struct FileEntry {
    size: u64,
    ext: String,
    attrs: Vec<u32>,
}

#[derive(Debug)]
pub enum PeerMsg {
    HPierceFirewall {
        token: u32,
    },
    SearchRequest {
        token: u32,
        query: String,
    },
    SearchReply {
        username: String,
        token: u32,
        results: HashMap<String, FileEntry>,
    },
}

pub struct PeerMsgCodec {
    cur_len: Option<usize>,
    cur_kind: Option<u32>,
}

impl PeerMsgCodec {
    pub fn new() -> Self {
        PeerMsgCodec {
            cur_len: None,
            cur_kind: None,
        }
    }
}

impl Encoder for PeerMsgCodec {
    type Item = PeerMsg;
    type Error = std::io::Error;

    fn encode(&mut self, msg: Self::Item, bytes: &mut BytesMut) -> Result<(), Self::Error> {
        use PeerMsg::*;

        let mut buf = BytesMut::new();

        println!("SEND PEER msg: {:?}", msg);

        match msg {
            HPierceFirewall { token } => {
                buf.put_u8(0);

                buf.put_u32_le(token);
            }
            SearchRequest { token, query } => {
                buf.put_u8(1);

                buf.put_u32_le(token);
                util::pack_string(&query, &mut buf);
            }
            _ => unreachable!(),
        }

        bytes.put_u32_le(buf.len() as u32);
        bytes.extend(buf);
        println!("send {:?}", bytes);

        Ok(())
    }
}

macro_rules! unp_msg {
    ($kind:ident, $name:ident) => {{
        println!("Unimplemented message {} ({})", stringify!($name), $kind);
        Ok(None)
    }};
}

macro_rules! unp_integer_msg {
    ($buf:ident, $kind:ident) => {{
        Ok(Some($kind($buf.split_to(4).into_buf().get_u32_le())))
    }};
}

macro_rules! unp_string_msg {
    ($buf:ident, $kind:ident) => {{
        println!("Unimplemented message {}", stringify!($kind));
        Ok(None)
    }};
}

macro_rules! unp_strings_msg {
    ($buf:ident, $kind:ident) => {{
        println!("Unimplemented message {}", stringify!($kind));
        Ok(None)
    }};
}

impl Decoder for PeerMsgCodec {
    type Item = PeerMsg;
    type Error = std::io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        use PeerMsg::*;

        if self.cur_len.is_none() {
            if buf.len() >= 4 {
                self.cur_len = Some(buf.split_to(4).into_buf().get_u32_le() as usize);
            } else {
                return Ok(None);
            }
        }
        if self.cur_kind.is_none() {
            if buf.len() >= 4 {
                self.cur_kind = Some(buf.split_to(4).into_buf().get_u32_le());
                println!("kind {:?}", self.cur_kind);
            } else {
                return Ok(None);
            }
        }

        // len contains u32 kind, but it was read already, so skip over it
        let len = self.cur_len.unwrap() - 4;
        let kind = self.cur_kind.unwrap();

        if buf.len() < len {
            return Ok(None);
        }

        // message is valid, do parse

        self.cur_len = None;
        self.cur_kind = None;

        let before = buf.len();

        println!("kind: {:?} len: {:?}", kind, len);

        let mut b = buf.split_to(len).into_buf();

        let result = match kind {
            9 => {
                let mut b = util::decompress(&mut b).into_buf();

                let username = util::get_string2(&mut b);
                let token = b.get_u32_le();
                let count = b.get_u32_le();
                let mut results = HashMap::new();

                for _ in 0..count {
                    buf.advance(1);
                    let filename = util::get_string2(&mut b);
                    let size = b.get_u64_le();
                    let ext = util::get_string2(&mut b);
                    let num_attrs = b.get_u32_le();
                    let mut attrs = vec![];
                    for _ in 0..num_attrs {
                        buf.advance(4);
                        attrs.push(b.get_u32_le());
                    }

                    results.insert(
                        filename,
                        FileEntry {
                            size: size,
                            ext: ext,
                            attrs: attrs,
                        },
                    );
                }

                Ok(Some(SearchReply {
                    username: username,
                    token: token,
                    results: results,
                }))
            }
            _ => {
                eprintln!("Unknown peer message {}", kind);
                Ok(None)
            }
        };

        result
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_hex_digest() {
        let source = BytesMut::from("0123456789abcdef");
        let result = hex_digest(&source, 16);

        assert_eq!("30313233343536373839616263646566\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0", result);
    }
}
