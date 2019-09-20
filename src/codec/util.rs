use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use std::net::Ipv4Addr;
use std::io::Read;

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

pub fn pack_ip(ip: &Ipv4Addr, bytes: &mut BytesMut) {
    bytes.extend(ip.octets().iter());
}

pub fn get_string(bytes: &mut BytesMut) -> String {
    let len = bytes.split_to(4).into_buf().get_u32_le() as usize;

    if bytes.len() < len {
        panic!()
    }

    let buf = bytes.split_to(len);
    String::from_utf8(buf.to_vec()).unwrap()
}

pub fn get_string2(bytes: &mut std::io::Cursor<BytesMut>) -> String {
    let len = bytes.get_u32_le() as usize;

    if bytes.get_ref().len() < len {
        panic!("{} {}", bytes.get_ref().len(), len);
    }

    let buf = Buf::take(Buf::by_ref(bytes), len).bytes().to_vec();
    let result = String::from_utf8(buf).unwrap();
    bytes.advance(len);
    result
}

pub fn get_ip(bytes: &mut BytesMut) -> Ipv4Addr {
    let buf = bytes.split_to(4);
    Ipv4Addr::new(buf[3], buf[2], buf[1], buf[0])
}

pub fn get_ip2(bytes: &mut std::io::Cursor<BytesMut>) -> Ipv4Addr {
    let d = bytes.get_u8();
    let c = bytes.get_u8();
    let b = bytes.get_u8();
    let a = bytes.get_u8();
    Ipv4Addr::new(a, b, c, d)
}

pub fn compress(bytes: &mut std::io::Cursor<BytesMut>) -> BytesMut  {
    let mut buf = vec![];
    let mut zlib = flate2::read::ZlibEncoder::new(bytes, flate2::Compression::default());
    zlib.read_to_end(&mut buf);
    BytesMut::from(buf)
}

pub fn decompress(bytes: &mut std::io::Cursor<BytesMut>) -> BytesMut  {
    let mut buf = vec![];
    let mut zlib = flate2::read::ZlibDecoder::new(bytes);
    zlib.read_to_end(&mut buf);
    BytesMut::from(buf)
}
