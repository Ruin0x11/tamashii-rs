use super::util;
use bytes::{Buf, BufMut, BytesMut, IntoBuf};
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4, SocketAddr};
use std::result::Result;
use tokio::codec::{Decoder, Encoder};
use num_derive::FromPrimitive;
use num_traits::FromPrimitive;
use log::*;

#[derive(Debug)]
pub enum SConnectToPeerKind {
    Peer,
    File,
    Distributed,
}

impl From<SConnectToPeerKind> for String {
    fn from(it: SConnectToPeerKind) -> String {
        match it {
            SConnectToPeerKind::Peer => "P",
            SConnectToPeerKind::File => "F",
            SConnectToPeerKind::Distributed => "D",
        }
        .into()
    }
}

impl From<String> for SConnectToPeerKind {
    fn from(it: String) -> SConnectToPeerKind {
        match it.as_ref() {
            "P" => SConnectToPeerKind::Peer,
            "F" => SConnectToPeerKind::File,
            "D" => SConnectToPeerKind::Distributed,
            _ => unreachable!(),
        }
    }
}

#[derive(Debug, FromPrimitive)]
pub enum CSetStatusStatus {
    Online = 1,
    Offline = 2,
}

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

    CHaveNoParents {
        have_parents: bool,
    },

    CSharedFoldersFiles {
        folders: u32,
        files: u32,
    },

    CSetStatus {
        status: CSetStatusStatus,
    },

    CRelatedSearch {
        query: String,
    },
    SRelatedSearch {
        query: String,
        related_searches: HashMap<String, i32>,
    },

    CParentIp {
        ip: Ipv4Addr
    },

    SConnectToPeer {
        username: String,
        kind: SConnectToPeerKind,
        ip: Ipv4Addr,
        port: u32,
        token: u32,
        use_obfuscation: bool,
        privileged: bool,
        obfuscated_port: u32,
    },

    SNetInfo {
        users: HashMap<String, (Ipv4Addr, u32)>
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
        buf.reserve(4096);

        debug!("SEND SERVER msg: {:?}", msg);

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
            CSharedFoldersFiles { folders, files } => {
                buf.put_u32_le(35);

                buf.put_u32_le(folders);
                buf.put_u32_le(files);
            }
            CHaveNoParents { have_parents } => {
                buf.put_u32_le(71);

                buf.put_u8(have_parents as u8);
            }
            CSetStatus { status } => {
                buf.put_u32_le(71);

                buf.put_u32_le(status as u32);
            }
            CRelatedSearch { query } => {
                buf.put_u32_le(153);

                util::pack_string(&query, &mut buf);
            }
            CParentIp { ip } => {
                buf.put_u32_le(153);

                util::pack_ip(&ip, &mut buf);
            }
            _ => unreachable!(),
        }

        bytes.put_u32_le(buf.len() as u32);
        bytes.extend(buf);
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
    ($b:ident, $kind:ident) => {{
        Ok(Some($kind($b.get_u32_le())))
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

        let mut b = buf.split_to(len).into_buf();

        let result = match kind {
            1 => {
                let success = b.get_u8() != 0;
                let greet = util::get_string2(&mut b);

                let (public_ip, unknown) = if success {
                    let public_ip = util::get_ip2(&mut b);
                    let unknown = b.get_u8() != 0;
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
                let username = util::get_string2(&mut b);
                let ip = util::get_ip2(&mut b);
                let port = b.get_u32_le();
                let use_obfuscation = b.get_u32_le() == 1;
                let obfuscated_port = b.get_u16_le();

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
            18 => {
                let username = util::get_string2(&mut b);
                let kind = util::get_string2(&mut b).into();
                let ip = util::get_ip2(&mut b);
                let port = b.get_u32_le();
                let token = b.get_u32_le();
                let use_obfuscation = b.get_u32_le() != 0;
                let privileged = b.get_u8() == 1;
                let obfuscated_port = b.get_u32_le();

                Ok(Some(SConnectToPeer {
                    username: username,
                    kind: kind,
                    ip: ip,
                    port: port,
                    token: token,
                    use_obfuscation: use_obfuscation,
                    privileged: privileged,
                    obfuscated_port: obfuscated_port,
                }))
            }
            22 => unp_msg!(kind, SPrivateMessage),
            23 => unp_integer_msg!(b, SAckPrivateMessage),
            26 => unp_msg!(kind, SFileSearch),
            28 => unp_integer_msg!(b, SSetStatus),
            32 => unp_msg!(kind, SPing),
            34 => unp_msg!(kind, SSendSpeed),
            // 35 => unp_msg!(kind, SSharedFoldersFiles),
            36 => unp_msg!(kind, SGetUserStats),
            41 => unp_msg!(kind, SKicked),
            42 => unp_msg!(kind, SUserSearch),
            51 => unp_string_msg!(b, SInterestAdd),
            52 => unp_string_msg!(b, SInterestRemove),
            54 => unp_msg!(kind, SGetRecommendations),
            56 => unp_msg!(kind, SGetGlobalRecommendations),
            57 => unp_msg!(kind, SUserInterests),
            64 => unp_msg!(kind, SRoomList),
            65 => unp_msg!(kind, SExactFileSearch),
            66 => unp_string_msg!(b, SGlobalMessage),
            69 => unp_strings_msg!(b, SPrivilegedUsers),
            71 => unp_msg!(kind, SHaveNoParents),
            73 => unp_msg!(kind, SParentIP),
            83 => unp_integer_msg!(b, SParentMinSpeed),
            84 => unp_integer_msg!(b, SParentSpeedRatio),
            86 => unp_integer_msg!(b, SParentInactivityTimeout),
            87 => unp_integer_msg!(b, SSearchInactivityTimeout),
            88 => unp_integer_msg!(b, SMinParentsInCache),
            90 => unp_integer_msg!(b, SDistribAliveInterval),
            91 => unp_string_msg!(b, SAddPrivileged),
            92 => unp_msg!(kind, SCheckPrivileges),
            // 93 => unp_msg!(kind, SSearchRequest),
            100 => unp_msg!(kind, SAcceptChildren),
            102 => {
                let mut users = HashMap::new();

                let count = b.get_u32_le();

                for _ in 0..count {
                    let user = util::get_string2(&mut b);
                    let ip = util::get_ip2(&mut b);
                    let port = b.get_u32_le();
                    users.insert(user, (ip, port));
                }

                Ok(Some(SNetInfo {
                    users: users
                }))
            }
            103 => unp_msg!(kind, SWishlistSearch),
            104 => unp_integer_msg!(b, SWishlistInterval),
            110 => unp_msg!(kind, SGetSimilarUsers),
            111 => unp_msg!(kind, SGetItemRecommendations),
            112 => unp_msg!(kind, SGetItemSimilarUsers),
            113 => unp_msg!(kind, SRoomTickers),
            114 => unp_msg!(kind, SRoomTickerAdd),
            115 => unp_msg!(kind, SRoomTickerRemove),
            116 => unp_msg!(kind, SSetRoomTicker),
            117 => unp_string_msg!(b, SInterestHatedAdd),
            118 => unp_string_msg!(b, SInterestHatedRemove),
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
            153 => {
                let query = util::get_string2(&mut b);
                let count = b.get_u32_le();
                let mut related_searches = HashMap::new();

                for _ in 0..count {
                    let term = util::get_string2(&mut b);
                    let score = b.get_i32_le();
                    related_searches.insert(term, score);
                }

                Ok(Some(SRelatedSearch {
                    query: query,
                    related_searches: related_searches,
                }))
            }
            1001 => unp_msg!(kind, SCannotConnect),

            _ => {
                warn!("Unknown server message {}", kind);
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
