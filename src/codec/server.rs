use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use std::net::Ipv4Addr;
use std::net::SocketAddr;
use std::result::Result;
use tokio::codec::{Decoder, Encoder};

#[derive(Debug)]
pub enum ServerMsg {
    CLogin {
        username: String,
        password: String,
    },
    SLogin {
        success: bool,
        greet: String,
        public_ip: Ipv4Addr,
        unknown: bool,
    },
}

static HEX_TABLE: &str = "0123456789abcdef";

fn hex_digest(source: &[u8], len: usize) -> BytesMut {
    let mut result = BytesMut::new();
    result.resize(len * 4 + 1, 0);

    for (i, ch) in source.iter().enumerate() {
        let ind1 = (ch >> 4) as usize;
        let ind2 = (ch & 0x0f) as usize;
        result[i * 2] = HEX_TABLE.chars().nth(ind1).unwrap() as u8;
        result[i * 2 + 1] = HEX_TABLE.chars().nth(ind2).unwrap() as u8;
    }
    result[len * 2] = 0;
    result
}

pub struct ServerMsgCodec {
    cur_kind: Option<u32>,
    cur_len: Option<usize>,
}

impl ServerMsgCodec {
    pub fn new() -> Self {
        ServerMsgCodec {
            cur_kind: None,
            cur_len: None,
        }
    }
}

fn pack_string(s: &str, bytes: &mut BytesMut, conv_slash: bool) {
    bytes.put_u32_le(s.len() as u32);
    if conv_slash {
        for ch in s.chars() {
            if ch == '/' {
                bytes.put_u8('\\' as u8);
            }
            else
            {
                bytes.put_u8(ch as u8);
            }
        }
    } else {
        bytes.extend(s.as_bytes());
    }
}

impl Encoder for ServerMsgCodec {
    type Item = ServerMsg;
    type Error = std::io::Error;

    fn encode(&mut self, msg: Self::Item, bytes: &mut BytesMut) -> Result<(), Self::Error> {
        use ServerMsg::*;

        let mut buf = BytesMut::new();

        match msg {
            CLogin { username, password } => {
                let src = format!("{}{}", username, password);
                let md5_digest = md5::compute(src);
                let digest = hex_digest(&*md5_digest, 16);

                buf.put_u32_le(1);

                pack_string(&username, &mut buf, false);
                pack_string(&password, &mut buf, false);
                buf.extend(&[183, 0, 0, 0]);
                buf.extend(digest);
                buf.extend(&[1, 0, 0, 0]);
            }
            _ => unreachable!(),
        }

        bytes.put_u32_le(buf.len() as u32);
        bytes.extend(buf);
        println!("send {:?}", bytes);

        Ok(())
    }
}

fn get_string(bytes: &mut BytesMut) -> String {
    let len = bytes.split_to(4).into_buf().get_u32_be() as usize;

    if bytes.len() < len {
        panic!()
    }

    let buf = bytes.split_to(len).into_buf();
    String::from_utf8(buf.bytes().to_vec()).unwrap()
}

fn get_ip(bytes: &mut BytesMut) -> Ipv4Addr {
    let mut buf = bytes.split_to(4);
    Ipv4Addr::new(buf[0], buf[1], buf[2], buf[3])
}

impl Decoder for ServerMsgCodec {
    type Item = ServerMsg;
    type Error = std::io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        use ServerMsg::*;

        if self.cur_kind.is_none() {
            if buf.len() >= 4 {
                self.cur_kind = Some(buf.split_to(4).into_buf().get_u32_be());
                println!("kind {:?}", self.cur_kind);
            } else {
                return Ok(None);
            }
        }
        if self.cur_len.is_none() {
            if buf.len() >= 4 {
                self.cur_len = Some(buf.split_to(4).into_buf().get_u32_be() as usize);
            } else {
                return Ok(None);
            }
        }

        let len = self.cur_len.unwrap();

        if buf.len() < len {
            return Ok(None);
        }

        match self.cur_kind {
            Some(1) => Ok(Some(SLogin {
                success: buf.split_to(1).into_buf().get_u8() != 0,
                greet: get_string(buf),
                public_ip: get_ip(buf),
                unknown: buf.split_to(1).into_buf().get_u8() != 0,
            })),
            _ => unreachable!(),
        }
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
