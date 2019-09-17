use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use std::net::Ipv4Addr;

pub fn pack_string(s: &str, bytes: &mut BytesMut) {
    bytes.put_u32_le(s.len() as u32);
    bytes.extend(s.as_bytes());
}

pub fn pack_path_string(s: &str, bytes: &mut BytesMut) {
    bytes.put_u32_le(s.len() as u32);
    for ch in s.chars() {
        if ch == '/' {
            bytes.put_u8('\\' as u8);
        } else {
            bytes.put_u8(ch as u8);
        }
    }
}

pub fn get_string(bytes: &mut BytesMut) -> String {
    let len = bytes.split_to(4).into_buf().get_u32_le() as usize;

    if bytes.len() < len {
        panic!()
    }

    let buf = bytes.split_to(len).into_buf();
    String::from_utf8(buf.bytes().to_vec()).unwrap()
}

pub fn get_ip(bytes: &mut BytesMut) -> Ipv4Addr {
    let mut buf = bytes.split_to(4);
    Ipv4Addr::new(buf[0], buf[1], buf[2], buf[3])
}
