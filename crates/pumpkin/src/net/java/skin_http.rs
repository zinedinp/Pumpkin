//! Serves Bedrock player skins as PNG so Java clients can fetch them.
//!
//! Java only loads skins from an HTTP URL in the `textures` profile property.
//! Requests arriving on the Java TCP port that look like HTTP are answered here
//! instead of going through the Minecraft handshake.

use std::io::Error;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use uuid::Uuid;

use crate::server::Server;

const MAX_HEADER_BYTES: usize = 8192;
const HEADER_TIMEOUT: Duration = Duration::from_millis(50);

/// If `stream` is an HTTP skin request, serve it and return `None`. Otherwise
/// give the stream back for the Java handshake.
pub async fn maybe_serve(
    mut stream: TcpStream,
    server: &Arc<Server>,
) -> Result<Option<TcpStream>, Error> {
    let mut peek = [0u8; 5];
    let Ok(Ok(n)) = tokio::time::timeout(HEADER_TIMEOUT, stream.peek(&mut peek)).await else {
        return Ok(Some(stream));
    };
    if n < 4 || (&peek[..4] != b"GET " && (n < 5 || &peek[..5] != b"HEAD ")) {
        return Ok(Some(stream));
    }

    let mut buf = Vec::new();
    loop {
        let mut chunk = [0u8; 1024];
        let n = match tokio::time::timeout(HEADER_TIMEOUT, stream.read(&mut chunk)).await {
            Ok(Ok(0)) | Err(_) => return Ok(None),
            Ok(Ok(n)) => n,
            Ok(Err(err)) => return Err(err),
        };
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Ok(None);
        }
    }

    let request = String::from_utf8_lossy(&buf);
    let mut parts = request.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    let is_head = method.eq_ignore_ascii_case("HEAD");

    let Some(uuid) = parse_skin_path(path) else {
        write_http(&mut stream, 404, b"text/plain", b"not found", is_head).await?;
        return Ok(None);
    };

    let Some(png) = server.java_skin_pngs.get(&uuid).map(|v| v.clone()) else {
        write_http(&mut stream, 404, b"text/plain", b"not found", is_head).await?;
        return Ok(None);
    };

    write_http(&mut stream, 200, b"image/png", &png, is_head).await?;
    Ok(None)
}

fn parse_skin_path(path: &str) -> Option<Uuid> {
    let path = path.split('?').next().unwrap_or(path);
    let rest = path.strip_prefix("/skin/")?.strip_suffix(".png")?;
    Uuid::parse_str(rest).ok()
}

async fn write_http(
    stream: &mut TcpStream,
    status: u16,
    content_type: &[u8],
    body: &[u8],
    head: bool,
) -> Result<(), Error> {
    let reason = match status {
        200 => "OK",
        _ => "Not Found",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        String::from_utf8_lossy(content_type),
        body.len()
    );
    stream.write_all(header.as_bytes()).await?;
    if !head {
        stream.write_all(body).await?;
    }
    let _ = stream.flush().await;
    Ok(())
}
