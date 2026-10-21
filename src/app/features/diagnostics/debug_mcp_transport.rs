use super::{Command, Reply, Request};
use eframe::egui;
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use std::time::{Duration, Instant};
pub(super) const MAX_BODY: usize = 16 * 1024;
const MAX_HEADER: usize = 8192;
pub(super) fn authorize(header: &str, port: u16, token: &str) -> Result<usize, String> {
    let mut lines = header.split("\r\n");
    if lines.next() != Some("POST /rpc HTTP/1.1") {
        return Err("Only POST /rpc is supported".into());
    }
    let mut fields = std::collections::HashMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let (key, value) = line.split_once(':').ok_or("Malformed header")?;
        if fields
            .insert(key.trim().to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return Err("Duplicate header".into());
        }
    }
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if !fields
        .get("host")
        .is_some_and(|h| hosts.iter().any(|x| x == h))
    {
        return Err("Nonlocal host".into());
    }
    if fields.get("authorization").copied() != Some(format!("Bearer {token}").as_str()) {
        return Err("Unauthorized".into());
    }
    if fields
        .get("origin")
        .is_some_and(|o| !hosts.iter().any(|h| format!("http://{h}") == *o))
    {
        return Err("Foreign origin".into());
    }
    if fields.contains_key("transfer-encoding")
        || fields.get("content-type").copied() != Some("application/json")
    {
        return Err("Expected bounded JSON".into());
    }
    let length = fields
        .get("content-length")
        .ok_or("Missing length")?
        .parse::<usize>()
        .map_err(|_| "Invalid length")?;
    if length == 0 || length > MAX_BODY {
        return Err("Body too large".into());
    }
    Ok(length)
}
pub(super) struct Server {
    stop: Arc<AtomicBool>,
    address: std::net::SocketAddr,
    done: mpsc::Receiver<()>,
}
impl Server {
    pub(super) fn start(
        ctx: egui::Context,
    ) -> Result<Option<(mpsc::Receiver<Request>, Self)>, String> {
        let Some(path) = std::env::var_os("CPU_AFFINITY_MCP_SESSION").map(std::path::PathBuf::from)
        else {
            return Ok(None);
        };
        let directory = path.parent().ok_or("Session directory required")?;
        let data = crate::app::adapters::storage::StorageAdapter::active_data_dir();
        let directory_real =
            std::fs::canonicalize(directory).map_err(|_| "MCP session directory unavailable")?;
        let data_real =
            std::fs::canonicalize(&data).map_err(|_| "MCP data directory unavailable")?;
        if directory_real != data_real || !directory.join(".cpu-affinity-debug-isolated").is_file()
        {
            return Err("MCP requires its isolated sidecar directory".into());
        }
        let token =
            std::env::var("CPU_AFFINITY_MCP_TOKEN").map_err(|_| "MCP session key missing")?;
        if token.len() != 64 || !token.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err("Invalid MCP session key".into());
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).map_err(|e| e.to_string())?;
        let address = listener.local_addr().map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::sync_channel(4);
        let (done_tx, done) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let ticket = json!({"schema":1,"url":format!("http://{address}/rpc"),"token":token,"pid":std::process::id()});
        std::fs::write(
            &path,
            serde_json::to_vec(&ticket).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        std::thread::Builder::new()
            .name("debug-mcp".into())
            .spawn(move || {
                let mut id = 0u64;
                for socket in listener.incoming() {
                    if flag.load(Ordering::Acquire) {
                        break;
                    }
                    if let Ok(mut stream) = socket {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
                        id = id.wrapping_add(1);
                        let result = serve(&mut stream, address.port(), &token, id, &tx, &ctx);
                        if let Err(error) = result {
                            let _ = send_json(&mut stream, 400, &json!({"ok":false,"error":error}));
                        }
                    }
                }
                let _ = done_tx.send(());
            })
            .map_err(|e| e.to_string())?;
        Ok(Some((
            rx,
            Self {
                stop,
                address,
                done,
            },
        )))
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(50));
        let _ = self.done.recv_timeout(Duration::from_millis(250));
    }
}
fn serve(
    stream: &mut TcpStream,
    port: u16,
    token: &str,
    id: u64,
    tx: &mpsc::SyncSender<Request>,
    ctx: &egui::Context,
) -> Result<(), String> {
    let mut header = Vec::with_capacity(1024);
    let read_started = Instant::now();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= MAX_HEADER || read_started.elapsed() > Duration::from_secs(2) {
            return Err("Header exceeded budget".into());
        }
        let mut byte = [0];
        stream.read_exact(&mut byte).map_err(|e| e.to_string())?;
        header.push(byte[0]);
    }
    let length = authorize(
        std::str::from_utf8(&header).map_err(|_| "Invalid header")?,
        port,
        token,
    )?;
    let mut body = vec![0; length];
    stream.read_exact(&mut body).map_err(|e| e.to_string())?;
    let command: Command =
        serde_json::from_slice(&body).map_err(|e| format!("Invalid debug command: {e}"))?;
    if let Command::Performance { seconds } = command {
        let value = match super::super::render_profile::performance(seconds) {
            Ok(data) => json!({"ok":true,"data":data}),
            Err(error) => json!({"ok":false,"error":error}),
        };
        return send_json(stream, 200, &value);
    }
    let (reply_tx, reply_rx) = mpsc::sync_channel(1);
    tx.try_send(Request {
        id,
        command,
        reply: reply_tx,
        deadline: Instant::now() + Duration::from_secs(8),
    })
    .map_err(|_| "GUI request queue full")?;
    ctx.request_repaint();
    match reply_rx
        .recv_timeout(Duration::from_secs(8))
        .map_err(|_| "GUI request timed out")?
    {
        Reply::Json(value) => send_json(stream, 200, &value),
        Reply::Image(image, frame) => {
            if image.pixels.len() > 4_000_000 {
                return Err("Screenshot exceeds pixel budget".into());
            }
            use image::ImageEncoder;
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
            let mut png = Vec::new();
            image::codecs::png::PngEncoder::new(&mut png)
                .write_image(
                    &rgba,
                    image.width() as u32,
                    image.height() as u32,
                    image::ExtendedColorType::Rgba8,
                )
                .map_err(|e| e.to_string())?;
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nX-App-Frame: {frame}\r\nConnection: close\r\n\r\n",png.len()).map_err(|e|e.to_string())?;
            stream.write_all(&png).map_err(|e| e.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn real_loopback_request_waits_for_gui_reply_and_preserves_exact_payload() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::sync_channel(4);
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            serve(
                &mut stream,
                address.port(),
                "test-key",
                7,
                &tx,
                &egui::Context::default(),
            )
            .unwrap();
        });
        let mut client = TcpStream::connect(address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let body = r#"{"action":"state"}"#;
        write!(client,"POST /rpc HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer test-key\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",body.len()).unwrap();
        let request = rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(
            request.command,
            Command::State {
                include_series: false
            }
        ));
        assert_eq!(request.id, 7);
        request
            .reply
            .send(Reply::Json(
                json!({"ok":true,"data":{"sample_seconds":12.25,"load":null}}),
            ))
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        let (_, body) = response.split_once("\r\n\r\n").unwrap();
        let value: Value = serde_json::from_str(body).unwrap();
        assert_eq!(value["data"]["sample_seconds"], 12.25);
        assert!(value["data"]["load"].is_null());
        thread.join().unwrap();
    }
}
fn send_json(stream: &mut TcpStream, status: u16, value: &Value) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("Reply exceeds JSON budget".into());
    }
    write!(stream,"HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).map_err(|e|e.to_string())?;
    stream.write_all(&bytes).map_err(|e| e.to_string())
}
