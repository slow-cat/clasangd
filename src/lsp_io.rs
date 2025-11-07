// use crate::prelude::*;
use serde_json::Value;
use std::io::ErrorKind;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
pub async fn read_lsp_message<R>(r: &mut R) -> io::Result<Value>
where
    R: AsyncRead + Unpin,
{
    let mut header = Vec::new();
    let mut buf = [0u8; 1];

    loop {
        let n = r.read(&mut buf).await?;
        if n == 0 {
            if header.is_empty() {
                return Err(io::Error::new(ErrorKind::UnexpectedEof, "eof"));
            } else {
                return Err(io::Error::new(ErrorKind::UnexpectedEof, "eof in header"));
            }
        }
        header.push(buf[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }

    let header_str = String::from_utf8_lossy(&header);
    let len = header_str
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("content-length"))
        .and_then(|l| l.split(':').nth(1))
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);

    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    let v: Value = serde_json::from_slice(&body)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e.to_string()))?;
    Ok(v)
}

pub async fn write_lsp_message<W>(w: &mut W, msg: &Value) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let body = serde_json::to_vec(msg)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e.to_string()))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    w.write_all(header.as_bytes()).await?;
    w.write_all(&body).await?;
    w.flush().await
}
