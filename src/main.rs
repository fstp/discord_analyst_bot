//#![feature(unboxed_closures)]

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::broadcast;

#[tokio::main]
async fn main() {
    let listener = TcpListener::bind("localhost:8080").await.unwrap();
    let (tx, _rx) = broadcast::channel(10);
    let mut client_count: usize = 0;

    loop {
        let (mut socket, addr) = listener.accept().await.unwrap();
        let tx = tx.clone();
        let mut rx = tx.subscribe();
        let id = client_count;
        client_count += 1;

        tokio::spawn(async move {
            let (reader, mut writer) = socket.split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();
            loop {
                tokio::select! {
                    result = reader.read_line(&mut line) => {
                        let num_bytes = result.unwrap();
                        if num_bytes == 0 {
                            println!("client {} disconnected", id);
                            break;
                        }
                        println!("client {} bytes_read: {}", id, num_bytes);
                        let msg = format!("Client {}: {}", id, line);
                        tx.send((msg, addr)).unwrap();
                        line.clear();
                    }
                    result = rx.recv() => {
                        let (msg, sender_addr) = result.unwrap();
                        if addr != sender_addr {
                            writer.write_all(msg.as_bytes()).await.unwrap();
                        }
                    }
                }
            }
        });
    }
}
