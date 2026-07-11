use std::fmt;

pub const SOCKET_PATH: &str = "/var/run/espresso.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Hold,
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusInfo {
    pub refcount: u32,
    pub sleep_disabled: bool,
    pub lid_closed: bool,
    pub pid: u32,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServerMsg {
    Ok,
    Status(StatusInfo),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcError {
    Malformed(String),
}

impl fmt::Display for IpcError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(s) => write!(f, "malformed IPC message: {s}"),
        }
    }
}

impl std::error::Error for IpcError {}

pub fn encode_client(m: &ClientMsg) -> String {
    match m {
        ClientMsg::Hold => "HOLD\n".to_string(),
        ClientMsg::Query => "QUERY\n".to_string(),
    }
}

pub fn decode_client(line: &str) -> Result<ClientMsg, IpcError> {
    match line.trim() {
        "HOLD" => Ok(ClientMsg::Hold),
        "QUERY" => Ok(ClientMsg::Query),
        other => Err(IpcError::Malformed(other.to_string())),
    }
}

pub fn encode_server(m: &ServerMsg) -> String {
    match m {
        ServerMsg::Ok => "OK\n".to_string(),
        ServerMsg::Status(s) => format!(
            "STATUS refcount={} sleep_disabled={} lid_closed={} pid={} version={}\n",
            s.refcount,
            s.sleep_disabled as u8,
            s.lid_closed as u8,
            s.pid,
            s.version,
        ),
    }
}

pub fn decode_server(line: &str) -> Result<ServerMsg, IpcError> {
    let line = line.trim();
    if line == "OK" {
        return Ok(ServerMsg::Ok);
    }
    let rest = line
        .strip_prefix("STATUS ")
        .ok_or_else(|| IpcError::Malformed(line.to_string()))?;

    let mut refcount = None;
    let mut sleep_disabled = None;
    let mut lid_closed = None;
    let mut pid = None;
    let mut version = None;

    for field in rest.split_whitespace() {
        let (k, v) = field
            .split_once('=')
            .ok_or_else(|| IpcError::Malformed(field.to_string()))?;
        match k {
            "refcount" => refcount = v.parse().ok(),
            "sleep_disabled" => sleep_disabled = Some(v == "1"),
            "lid_closed" => lid_closed = Some(v == "1"),
            "pid" => pid = v.parse().ok(),
            "version" => version = Some(v.to_string()),
            _ => {}
        }
    }

    Ok(ServerMsg::Status(StatusInfo {
        refcount: refcount.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
        sleep_disabled: sleep_disabled.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
        lid_closed: lid_closed.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
        pid: pid.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
        version: version.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_round_trip() {
        for m in [ClientMsg::Hold, ClientMsg::Query] {
            let line = encode_client(&m);
            assert_eq!(decode_client(line.trim_end()), Ok(m));
        }
    }

    #[test]
    fn status_round_trip() {
        let info = StatusInfo {
            refcount: 2,
            sleep_disabled: true,
            lid_closed: false,
            pid: 4821,
            version: "0.2.0".into(),
        };
        let line = encode_server(&ServerMsg::Status(info.clone()));
        assert_eq!(decode_server(line.trim_end()), Ok(ServerMsg::Status(info)));
    }

    #[test]
    fn ok_round_trip() {
        let line = encode_server(&ServerMsg::Ok);
        assert_eq!(decode_server(line.trim_end()), Ok(ServerMsg::Ok));
    }

    #[test]
    fn malformed_client_rejected() {
        assert!(matches!(decode_client("NONSENSE"), Err(IpcError::Malformed(_))));
    }
}
