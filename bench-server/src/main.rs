/// Minimal raw TCP HTTP server for benchmarking.
/// Returns fixed-size response with zero overhead.
/// Usage: bench-server [port] [response-body-size-bytes]
use std::env;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let port: u16 = env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(8080);
    let body_size: usize = env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1024);

    // Pre-build response bytes ONCE
    let body = "x".repeat(body_size);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n{}",
        body_size, body
    );
    let response_bytes: &'static [u8] = Box::leak(response.into_bytes().into_boxed_slice());

    let listener = TcpListener::bind(format!("0.0.0.0:{}", port)).await.unwrap();
    eprintln!("bench-server listening on :{} (body={}B)", port, body_size);

    loop {
        let (stream, _) = listener.accept().await.unwrap();
        let _ = stream.set_nodelay(true);
        tokio::spawn(async move {
            handle(stream, response_bytes).await;
        });
    }
}

async fn handle(mut stream: tokio::net::TcpStream, response: &[u8]) {
    let mut buf = [0u8; 4096];
    loop {
        // Read until we have a complete HTTP request (\r\n\r\n)
        let mut total = 0;
        let mut found = false;
        while !found {
            match stream.read(&mut buf[total..]).await {
                Ok(0) => return,
                Ok(n) => {
                    total += n;
                    // Scan for \r\n\r\n
                    let start = if total > n + 3 { total - n - 3 } else { 0 };
                    for i in start..total.saturating_sub(3) {
                        if buf[i] == b'\r' && buf[i+1] == b'\n' && buf[i+2] == b'\r' && buf[i+3] == b'\n' {
                            found = true;
                            break;
                        }
                    }
                    if total >= buf.len() {
                        found = true; // buffer full, assume request complete
                    }
                }
                Err(_) => return,
            }
        }

        // Write pre-built response (single syscall)
        if stream.write_all(response).await.is_err() {
            return;
        }
    }
}
