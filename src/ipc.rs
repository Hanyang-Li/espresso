use std::fmt;

pub const SOCKET_PATH: &str = "/var/run/espresso.sock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClientMsg {
    Hold { pid: u32, command: String },
    Query,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub pid: u32,
    pub command: String,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusInfo {
    pub sessions: Vec<SessionInfo>,
    pub pid: u32,
    pub version: String,
}

/// Replace line-breaking bytes so a value stays on its own IPC line.
fn sanitize_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
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
        ClientMsg::Hold { pid, command } => {
            format!("HOLD pid={pid} cmd={}\n", sanitize_line(command))
        }
        ClientMsg::Query => "QUERY\n".to_string(),
    }
}

pub fn decode_client(line: &str) -> Result<ClientMsg, IpcError> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line == "QUERY" {
        return Ok(ClientMsg::Query);
    }
    if line == "HOLD" {
        // Bare HOLD from an older client: no metadata available.
        return Ok(ClientMsg::Hold { pid: 0, command: String::new() });
    }
    if let Some(rest) = line.strip_prefix("HOLD ") {
        let (pid_part, cmd) = rest
            .split_once(" cmd=")
            .ok_or_else(|| IpcError::Malformed(line.to_string()))?;
        let pid = pid_part
            .strip_prefix("pid=")
            .and_then(|v| v.parse().ok())
            .ok_or_else(|| IpcError::Malformed(line.to_string()))?;
        return Ok(ClientMsg::Hold { pid, command: cmd.to_string() });
    }
    Err(IpcError::Malformed(line.to_string()))
}

pub fn encode_server(m: &ServerMsg) -> String {
    match m {
        ServerMsg::Ok => "OK\n".to_string(),
        ServerMsg::Status(s) => {
            let mut out = format!(
                "STATUS pid={} sessions={} version={}\n",
                s.pid,
                s.sessions.len(),
                sanitize_line(&s.version),
            );
            for sess in &s.sessions {
                out.push_str(&format!(
                    "SESSION pid={} uptime={} cmd={}\n",
                    sess.pid,
                    sess.uptime_secs,
                    sanitize_line(&sess.command),
                ));
            }
            out
        }
    }
}

pub fn decode_server(input: &str) -> Result<ServerMsg, IpcError> {
    let mut lines = input.lines();
    let first = lines.next().unwrap_or("").trim_end();
    if first == "OK" {
        return Ok(ServerMsg::Ok);
    }
    let rest = first
        .strip_prefix("STATUS ")
        .ok_or_else(|| IpcError::Malformed(first.to_string()))?;

    // `version` is the last field on the header line and may contain spaces.
    let (fields, version) = rest
        .split_once(" version=")
        .ok_or_else(|| IpcError::Malformed(first.to_string()))?;

    let mut pid = None;
    let mut count = None;
    for field in fields.split_whitespace() {
        let (k, v) = field
            .split_once('=')
            .ok_or_else(|| IpcError::Malformed(field.to_string()))?;
        match k {
            "pid" => pid = v.parse().ok(),
            "sessions" => count = v.parse::<usize>().ok(),
            _ => return Err(IpcError::Malformed(field.to_string())),
        }
    }
    let pid = pid.ok_or_else(|| IpcError::Malformed(first.to_string()))?;
    let count = count.ok_or_else(|| IpcError::Malformed(first.to_string()))?;

    let mut sessions = Vec::new();
    for line in lines {
        let sl = line
            .strip_prefix("SESSION ")
            .ok_or_else(|| IpcError::Malformed(line.to_string()))?;
        // `cmd` is the last field and may contain spaces.
        let (sf, cmd) = sl
            .split_once(" cmd=")
            .ok_or_else(|| IpcError::Malformed(line.to_string()))?;
        let mut spid = None;
        let mut uptime = None;
        for field in sf.split_whitespace() {
            let (k, v) = field
                .split_once('=')
                .ok_or_else(|| IpcError::Malformed(field.to_string()))?;
            match k {
                "pid" => spid = v.parse().ok(),
                "uptime" => uptime = v.parse().ok(),
                _ => return Err(IpcError::Malformed(field.to_string())),
            }
        }
        sessions.push(SessionInfo {
            pid: spid.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
            uptime_secs: uptime.ok_or_else(|| IpcError::Malformed(line.to_string()))?,
            command: cmd.to_string(),
        });
    }
    if sessions.len() != count {
        return Err(IpcError::Malformed(format!(
            "session count {count} != {} SESSION lines",
            sessions.len()
        )));
    }

    Ok(ServerMsg::Status(StatusInfo {
        sessions,
        pid,
        version: version.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_round_trip() {
        for m in [
            ClientMsg::Hold { pid: 4821, command: "espresso -- sleep 100".into() },
            ClientMsg::Hold { pid: 7, command: "espresso -- echo 你好 世界".into() },
            ClientMsg::Query,
        ] {
            let line = encode_client(&m);
            assert_eq!(decode_client(&line), Ok(m));
        }
    }

    #[test]
    fn bare_hold_still_decodes() {
        assert_eq!(
            decode_client("HOLD"),
            Ok(ClientMsg::Hold { pid: 0, command: String::new() })
        );
    }

    #[test]
    fn ok_round_trip() {
        let line = encode_server(&ServerMsg::Ok);
        assert_eq!(decode_server(line.trim_end()), Ok(ServerMsg::Ok));
    }

    #[test]
    fn malformed_client_rejected() {
        assert!(matches!(decode_client("NONSENSE"), Err(IpcError::Malformed(_))));
        assert!(matches!(decode_client("HOLD cmd=x"), Err(IpcError::Malformed(_))));
    }

    #[test]
    fn status_round_trip_no_sessions() {
        let info = StatusInfo {
            sessions: vec![],
            pid: 4821,
            version: "0.2.2".into(),
        };
        let line = encode_server(&ServerMsg::Status(info.clone()));
        assert_eq!(decode_server(&line), Ok(ServerMsg::Status(info)));
    }

    #[test]
    fn status_round_trip_with_sessions_incl_spaces_and_cjk() {
        let info = StatusInfo {
            sessions: vec![
                SessionInfo { pid: 12346, command: "espresso -- sleep 100".into(), uptime_secs: 192 },
                SessionInfo { pid: 12888, command: "espresso -- echo 你好 世界".into(), uptime_secs: 45 },
            ],
            pid: 700,
            version: "0.2.2 debug build".into(),
        };
        let line = encode_server(&ServerMsg::Status(info.clone()));
        assert_eq!(decode_server(&line), Ok(ServerMsg::Status(info)));
    }

    #[test]
    fn status_session_count_mismatch_rejected() {
        // Header claims 1 session but no SESSION line follows.
        assert!(matches!(
            decode_server("STATUS pid=1 sessions=1 version=0.1"),
            Err(IpcError::Malformed(_))
        ));
    }

    #[test]
    fn status_missing_prefix_rejected() {
        assert!(matches!(
            decode_server("pid=1 sessions=0 version=0.1"),
            Err(IpcError::Malformed(_))
        ));
    }

    #[test]
    fn status_missing_field_rejected() {
        assert!(matches!(
            decode_server("STATUS sessions=0 version=0.1"),
            Err(IpcError::Malformed(_))
        ));
    }

    #[test]
    fn status_unknown_field_rejected() {
        assert!(matches!(
            decode_server("STATUS pid=1 sessions=0 bogus=1 version=0.1"),
            Err(IpcError::Malformed(_))
        ));
    }
}
