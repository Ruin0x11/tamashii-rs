use super::util;
use byteorder::{ByteOrder, LittleEndian};
use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use log::*;
use rand::Rng;
use std::collections::HashMap;
use std::io::Read;
use std::net::Ipv4Addr;
use std::result::Result;
use tokio::codec::{Decoder, Encoder};

use super::structs::FileEntry;

#[derive(Clone, Debug, PartialEq, Eq)]
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
        results: Vec<FileEntry>,
        slots_free: bool,
        average_speed: u32,
        queue_length: u64,
        locked_results: Option<Vec<FileEntry>>,
    },
}

pub struct PeerMsgCodec {
    cur_key: Option<BytesMut>,
    cur_len: Option<usize>,
    cur_kind: Option<u32>,
    use_obfuscation: bool,
}

impl PeerMsgCodec {
    pub fn new(use_obfuscation: bool) -> Self {
        PeerMsgCodec {
            cur_key: None,
            cur_len: None,
            cur_kind: None,
            use_obfuscation: use_obfuscation,
        }
    }
}

fn generate_obfuscation_key() -> BytesMut {
    let mut result = BytesMut::new();
    let mut rng = rand::thread_rng();
    for _ in 0..4 {
        result.put_u8(rng.gen());
    }
    result
}

fn rotate_key(key: &mut BytesMut) {
    let mask = 0b00111111;
    let c = 31u32 & mask;

    let f = (!c) & mask;

    let key_orig =
        key[0] as u32 + ((key[1] as u32) << 8) + ((key[2] as u32) << 16) + ((key[3] as u32) << 24);
    let key_rotated = (key_orig >> c) | (key_orig.wrapping_shl(f + 1));

    key[0] = (key_rotated & 0xff) as u8;
    key[1] = ((key_rotated >> 8) & 0xff) as u8;
    key[2] = ((key_rotated >> 16) & 0xff) as u8;
    key[3] = ((key_rotated >> 24) & 0xff) as u8;
}

fn obfuscate(src: &mut BytesMut, key: &mut BytesMut, len: usize) {
    let key_size = 4;

    for i in 0..len {
        let key_pos = i % key_size;
        if key_pos == 0 {
            rotate_key(key);
        }
        src[i] = src[i] ^ key[key_pos];
    }
}

impl Encoder for PeerMsgCodec {
    type Item = PeerMsg;
    type Error = std::io::Error;

    fn encode(&mut self, msg: Self::Item, bytes: &mut BytesMut) -> Result<(), Self::Error> {
        use PeerMsg::*;

        let mut buf = BytesMut::new();
        buf.reserve(4096);
        let kind;
        let mut use_u8 = false;

        debug!("SEND PEER msg: {:?}", msg);

        match msg {
            HPierceFirewall { token } => {
                kind = 0;
                use_u8 = true;

                buf.put_u32_le(token);
            }
            SearchRequest { token, query } => {
                kind = 1;

                buf.put_u32_le(token);
                util::pack_string(&query, &mut buf);
            }
            _ => unreachable!(),
        }

        if self.use_obfuscation && !use_u8 {
            let mut key = generate_obfuscation_key();
            bytes.extend(key.clone());

            let mut len_buf = BytesMut::new();
            len_buf.resize(4, 0);
            let len = buf.len();

            LittleEndian::write_u32(&mut len_buf, len as u32);
            obfuscate(&mut len_buf, &mut key, 4);
            bytes.extend(len_buf);

            let mut kind_buf = BytesMut::new();
            let kind_len;
            if use_u8 {
                kind_buf.resize(1, 0);
                kind_buf.put_u8(kind as u8);
                kind_len = 1;
            } else {
                kind_buf.resize(4, 0);
                kind_buf.put_u32_le(kind);
                kind_len = 4;
            }

            obfuscate(&mut kind_buf, &mut key, kind_len);
            bytes.extend(kind_buf);

            obfuscate(&mut buf, &mut key, len);
            bytes.extend(buf);

            info!("obfuscated");
        } else {
            bytes.put_u32_le(buf.len() as u32);
            if use_u8 {
                bytes.put_u8(kind as u8);
            } else {
                bytes.put_u32_le(kind);
            }
            bytes.extend(buf);
        }

        trace!("send {:?}", bytes);

        Ok(())
    }
}

macro_rules! unp_msg {
    ($kind:ident, $name:ident) => {{
        warn!("Unimplemented message {} ({})", stringify!($name), $kind);
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
        warn!("Unimplemented message {}", stringify!($kind));
        Ok(None)
    }};
}

macro_rules! unp_strings_msg {
    ($buf:ident, $kind:ident) => {{
        warn!("Unimplemented message {}", stringify!($kind));
        Ok(None)
    }};
}

fn unpack_file_entry_u64(b: &mut std::io::Cursor<BytesMut>) -> Option<FileEntry> {
    b.advance(1);
    let filename = util::get_string2(b);
    let size = b.get_u64_le(); // some clients may use u32
    let ext = util::get_string2(b);
    let num_attrs = b.get_u32_le();
    let mut attrs = vec![];
    for _ in 0..num_attrs {
        if b.remaining() < 4 {
            // malformed message; clear buffer
            b.advance(b.remaining());
            return None;
        }
        b.advance(4);
        attrs.push(b.get_u32_le());
    }

    Some(FileEntry {
        filename: filename,
        size: size,
        ext: ext,
        attrs: attrs,
    })
}

fn unpack_file_entry_u32(b: &mut std::io::Cursor<BytesMut>) -> Option<FileEntry> {
    b.advance(1);
    let filename = util::get_string2(b);
    let size = b.get_u32_le(); // some clients may use u32
    let ext = util::get_string2(b);
    let num_attrs = b.get_u32_le();
    let mut attrs = vec![];
    for _ in 0..num_attrs {
        if b.remaining() < 4 {
            // malformed message; clear buffer
            b.advance(b.remaining());
            return None;
        }
        b.advance(4);
        attrs.push(b.get_u32_le());
    }

    Some(FileEntry {
        filename: filename,
        size: size as u64,
        ext: ext,
        attrs: attrs,
    })
}

impl Decoder for PeerMsgCodec {
    type Item = PeerMsg;
    type Error = std::io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        use PeerMsg::*;

        let mut handshake = false;

        if self.use_obfuscation && self.cur_key.is_none() {
            if buf.len() >= 4 {
                self.cur_key = Some(buf.split_to(4));
                info!("key: {:?}", self.cur_key);
            } else {
                return Ok(None);
            }
        }
        if self.cur_len.is_none() {
            if buf.len() >= 4 {
                let mut data = buf.split_to(4);

                if self.use_obfuscation {
                    obfuscate(&mut data, self.cur_key.as_mut().unwrap(), 4);
                }

                self.cur_len = Some(data.into_buf().get_u32_le() as usize);
                info!("len: {:?}", self.cur_key);
            } else {
                return Ok(None);
            }
        }
        if self.cur_kind.is_none() {
            if buf.len() >= 4 {
                // handshakes use u8 while peer messages use u32 little endian.
                let mut data = buf.split_to(1);
                handshake = data[0] == 0 || data[0] == 1 || data[0] == 2 || data[0] == 3;

                if handshake {
                    data.extend(&[0, 0, 0]);
                } else {
                    let rest = buf.split_to(3);
                    data.extend(rest);
                }
                info!("kind: {:?}", self.cur_kind);

                self.cur_kind = Some(data.into_buf().get_u32_le());
            } else {
                return Ok(None);
            }
        }

        // len contains u8/u32 kind, but it was read already, so skip over it
        let read_size = if handshake { 1 } else { 4 };

        let len = self.cur_len.unwrap() - read_size;
        let kind = self.cur_kind.unwrap();

        if buf.len() < len {
            return Ok(None);
        }

        // message is valid, do parse

        self.cur_len = None;
        self.cur_kind = None;

        trace!("kind: {:?} len: {:?}", kind, len);

        let mut data = buf.split_to(len);

        if self.use_obfuscation {
            let mut key = self.cur_key.clone().unwrap();
            self.cur_key = None;

            info!("deobfuscated");
            obfuscate(&mut data, &mut key, len);
        }

        let mut b = data.into_buf();

        let result = match kind {
            0 => {
                let token = b.get_u32_le();

                Ok(Some(HPierceFirewall { token: token }))
            }
            1 => {
                let token = b.get_u32_le();
                let query = util::get_string2(&mut b);

                Ok(Some(SearchRequest {
                    token: token,
                    query: query,
                }))
            }
            9 => {
                let mut b = util::decompress(&mut b).into_buf();

                // message may be malformed because some clients use u32 instead
                // of u64 for size. use museek's error handling and reparse if
                // buffer is exhausted.
                let backup = b.clone();
                let mut malformed = false;

                let username = util::get_string2(&mut b);
                let token = b.get_u32_le();
                let count = b.get_u32_le();
                let mut results = vec![];
                let mut locked_results = None;

                for _ in 0..count {
                    if b.remaining() < 16 {
                        // malformed message; clear current buffer and reparse with filesize as u32
                        malformed = true;
                        b.advance(b.remaining());
                        break;
                    }

                    unpack_file_entry_u64(&mut b).map(|e| results.push(e));
                }

                let slots_free = b.get_u8() != 0;
                let average_speed = b.get_u32_le();

                // some clients use u32 instead of u64
                let queue_length = if b.remaining() >= 8 {
                    b.get_u64_le()
                } else {
                    b.get_u32_le() as u64
                };

                // newer clients may return locked files
                if b.remaining() >= 4 {
                    let mut _locked_results = vec![];
                    let count = b.get_u32_le();

                    for _ in 0..count {
                        if b.remaining() < 16 {
                            // malformed message; clear buffer
                            b.advance(b.remaining());
                            break;
                        }

                        unpack_file_entry_u64(&mut b).map(|e| _locked_results.push(e));
                    }

                    locked_results = Some(_locked_results);
                }

                if malformed {
                    b = backup;
                    results.clear();

                    for _ in 0..count {
                        if !b.has_remaining() {
                            // malformed message
                            break;
                        }
                        unpack_file_entry_u32(&mut b).map(|e| results.push(e));
                    }
                }

                Ok(Some(SearchReply {
                    username: username,
                    token: token,
                    results: results,
                    slots_free: slots_free,
                    average_speed: average_speed,
                    queue_length: queue_length,
                    locked_results: locked_results,
                }))
            }
            _ => {
                warn!("Unknown peer message {}", kind);
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
    fn test_rotate_key() {
        let mut key = BytesMut::new();
        key.extend(&[221, 1, 148, 189]);

        rotate_key(&mut key);

        assert_eq!(&[187, 3, 40, 123], &*key);
    }

    #[test]
    fn test_obfuscation_encode() {
        let mut source = BytesMut::from("test message");
        let mut key = BytesMut::new();
        key.extend(&[221, 1, 148, 189]);

        let len = source.len();
        obfuscate(&mut source, &mut key, len);

        assert_eq!(
            &[207, 102, 91, 15, 86, 106, 53, 133, 158, 111, 199, 137],
            &*source
        );
    }

    #[test]
    fn test_obfuscation_decode() {
        let mut source = BytesMut::new();
        source.extend(&[207, 102, 91, 15, 86, 106, 53, 133, 158, 111, 199, 137]);
        let mut key = BytesMut::new();
        key.extend(&[221, 1, 148, 189]);

        let len = source.len();
        obfuscate(&mut source, &mut key, len);

        assert_eq!(&String::from_utf8(source.to_vec()).unwrap(), "test message");
    }

    #[test]
    fn test_obfuscated_encode_decode() {
        let msg = PeerMsg::HPierceFirewall { token: 1234 };

        let mut bytes = BytesMut::new();
        bytes.reserve(1024);

        let mut codec = PeerMsgCodec::new(true);

        codec.encode(msg.clone(), &mut bytes).unwrap();
        let result = codec.decode(&mut bytes).unwrap().unwrap();

        assert_eq!(msg, result);
    }

    #[test]
    fn test_decode2() {
        let mut v = vec![];
        let mut codec = PeerMsgCodec::new(true);

        let mut f = std::fs::File::open("/home/ruin/build/zlib").unwrap();
        f.read_to_end(&mut v);
        let mut bytes = BytesMut::from(v);

        let result = codec.decode(&mut bytes).unwrap().unwrap();
    }
}
