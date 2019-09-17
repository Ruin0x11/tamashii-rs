use super::util;
use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use std::net::Ipv4Addr;
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
        public_ip: Option<Ipv4Addr>,
        unknown: Option<bool>,
    },

    CSetListenPort {
        port: u32,
        use_obfuscation: bool,
    },

    CGetPeerAddress {
        username: String,
    },
    SGetPeerAddress {
        username: String,
        ip: Ipv4Addr,
        port: u32,
        use_obfuscation: bool,
        obfuscated_port: u16,
    },

    CFileSearch {
        token: u32,
        query: String,
    },

    CRelatedSearch {
        query: String,
    },

    SAckPrivateMessage(u32),
    SSetStatus(u32),
    SParentMinSpeed(u32),
    SParentSpeedRatio(u32),
    SParentInactivityTimeout(u32),
    SSearchInactivityTimeout(u32),
    SMinParentsInCache(u32),
    SDistribAliveInterval(u32),
    SWishlistInterval(u32),
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
    cur_len: Option<usize>,
    cur_kind: Option<u32>,
}

impl ServerMsgCodec {
    pub fn new() -> Self {
        ServerMsgCodec {
            cur_len: None,
            cur_kind: None,
        }
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

                util::pack_string(&username, &mut buf);
                util::pack_string(&password, &mut buf);
                buf.extend(&[183, 0, 0, 0]);
                buf.extend(digest);
                buf.extend(&[1, 0, 0, 0]);
            }
            CSetListenPort {
                port,
                use_obfuscation,
            } => {
                buf.put_u32_le(2);

                buf.put_u32_le(port);
                buf.put_u32_le(use_obfuscation as u32);
            }
            CGetPeerAddress { username } => {
                buf.put_u32_le(3);

                util::pack_string(&username, &mut buf)
            }
            CFileSearch { token, query } => {
                buf.put_u32_le(26);

                buf.put_u32_le(token);
                util::pack_string(&query, &mut buf);
            }
            CRelatedSearch { query } => {
                buf.put_u32_le(153);

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

impl Decoder for ServerMsgCodec {
    type Item = ServerMsg;
    type Error = std::io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        use ServerMsg::*;

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

        let result = match kind {
            1 => {
                let success = buf.split_to(1).into_buf().get_u8() != 0;
                let greet = util::get_string(buf);

                let (public_ip, unknown) = if success {
                    let public_ip = util::get_ip(buf);
                    let unknown = buf.split_to(1).into_buf().get_u8() != 0;
                    (Some(public_ip), Some(unknown))
                } else {
                    (None, None)
                };

                Ok(Some(SLogin {
                    success: success,
                    greet: greet,
                    public_ip: public_ip,
                    unknown: unknown,
                }))
            }
            // 2 => unp_msg!(kind, SSetListenPort),
            3 => {
                let username = util::get_string(buf);
                let ip = util::get_ip(buf);
                let port = buf.split_to(1).into_buf().get_u32_le();
                let use_obfuscation = buf.split_to(1).into_buf().get_u32_le() == 1;
                let obfuscated_port = buf.split_to(1).into_buf().get_u16_le();

                Ok(Some(SGetPeerAddress {
                    username: username,
                    ip: ip,
                    port: port,
                    use_obfuscation: use_obfuscation,
                    obfuscated_port: obfuscated_port,
                }))
            }
            5 => unp_msg!(kind, SAddUser),
            7 => unp_msg!(kind, SGetStatus),
            13 => unp_msg!(kind, SSayRoom),
            14 => unp_msg!(kind, SJoinRoom),
            15 => unp_string_msg!(buf, SLeaveRoom),
            16 => unp_msg!(kind, SUserJoinedRoom),
            17 => unp_msg!(kind, SUserLeftRoom),
            18 => unp_msg!(kind, SConnectToPeer),
            22 => unp_msg!(kind, SPrivateMessage),
            23 => unp_integer_msg!(buf, SAckPrivateMessage),
            26 => unp_msg!(kind, SFileSearch),
            28 => unp_integer_msg!(buf, SSetStatus),
            32 => unp_msg!(kind, SPing),
            34 => unp_msg!(kind, SSendSpeed),
            35 => unp_msg!(kind, SSharedFoldersFiles),
            36 => unp_msg!(kind, SGetUserStats),
            41 => unp_msg!(kind, SKicked),
            42 => unp_msg!(kind, SUserSearch),
            51 => unp_string_msg!(buf, SInterestAdd),
            52 => unp_string_msg!(buf, SInterestRemove),
            54 => unp_msg!(kind, SGetRecommendations),
            56 => unp_msg!(kind, SGetGlobalRecommendations),
            57 => unp_msg!(kind, SUserInterests),
            64 => unp_msg!(kind, SRoomList),
            65 => unp_msg!(kind, SExactFileSearch),
            66 => unp_string_msg!(buf, SGlobalMessage),
            69 => unp_strings_msg!(buf, SPrivilegedUsers),
            71 => unp_msg!(kind, SHaveNoParents),
            73 => unp_msg!(kind, SParentIP),
            83 => unp_integer_msg!(buf, SParentMinSpeed),
            84 => unp_integer_msg!(buf, SParentSpeedRatio),
            86 => unp_integer_msg!(buf, SParentInactivityTimeout),
            87 => unp_integer_msg!(buf, SSearchInactivityTimeout),
            88 => unp_integer_msg!(buf, SMinParentsInCache),
            90 => unp_integer_msg!(buf, SDistribAliveInterval),
            91 => unp_string_msg!(buf, SAddPrivileged),
            92 => unp_msg!(kind, SCheckPrivileges),
            // 93 => unp_msg!(kind, SSearchRequest),
            100 => unp_msg!(kind, SAcceptChildren),
            102 => unp_msg!(kind, SNetInfo),
            103 => unp_msg!(kind, SWishlistSearch),
            104 => unp_integer_msg!(buf, SWishlistInterval),
            110 => unp_msg!(kind, SGetSimilarUsers),
            111 => unp_msg!(kind, SGetItemRecommendations),
            112 => unp_msg!(kind, SGetItemSimilarUsers),
            113 => unp_msg!(kind, SRoomTickers),
            114 => unp_msg!(kind, SRoomTickerAdd),
            115 => unp_msg!(kind, SRoomTickerRemove),
            116 => unp_msg!(kind, SSetRoomTicker),
            117 => unp_string_msg!(buf, SInterestHatedAdd),
            118 => unp_string_msg!(buf, SInterestHatedRemove),
            120 => unp_msg!(kind, SRoomSearch),
            121 => unp_msg!(kind, SSendUploadSpeed),
            122 => unp_msg!(kind, SUserPrivileges),
            123 => unp_msg!(kind, SGivePrivileges),
            124 => unp_msg!(kind, SNotifyPrivileges),
            125 => unp_msg!(kind, SAckNotifyPrivileges),
            126 => unp_msg!(kind, SBranchLevel),
            127 => unp_msg!(kind, SBranchRoot),
            129 => unp_msg!(kind, SChildDepth),
            133 => unp_msg!(kind, SPrivRoomAlterableMembers),
            134 => unp_msg!(kind, SPrivRoomAddUser),
            135 => unp_msg!(kind, SPrivRoomRemoveUser),
            136 => unp_msg!(kind, SPrivRoomDismember),
            137 => unp_msg!(kind, SPrivRoomDisown),
            138 => unp_msg!(kind, SPrivRoomUnknown138),
            139 => unp_msg!(kind, SPrivRoomAdded),
            140 => unp_msg!(kind, SPrivRoomRemoved),
            141 => unp_msg!(kind, SPrivRoomToggle),
            142 => unp_msg!(kind, SNewPassword),
            143 => unp_msg!(kind, SPrivRoomAddOperator),
            144 => unp_msg!(kind, SPrivRoomRemoveOperator),
            145 => unp_msg!(kind, SPrivRoomOperatorAdded),
            146 => unp_msg!(kind, SPrivRoomOperatorRemoved),
            148 => unp_msg!(kind, SPrivRoomAlterableOperators),
            149 => unp_msg!(kind, SMessageUsers),
            150 => unp_msg!(kind, SAskPublicChat),
            151 => unp_msg!(kind, SStopPublicChat),
            152 => unp_msg!(kind, SPublicChat),
            153 => unp_msg!(kind, SRelatedSearch),
            1001 => unp_msg!(kind, SCannotConnect),

            _ => {
                eprintln!("Unknown server message {}", kind);
                Ok(None)
            }
        };

        println!("get msg: {:?}", result);

        let consumed = before - buf.len();
        assert!(consumed <= len, "len: {} consumed: {}", len, consumed);
        let left = len - consumed;
        buf.advance(left);

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
