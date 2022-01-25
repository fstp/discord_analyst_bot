//#![feature(unboxed_closures)]

use dialoguer::Input;
use std::thread;
use console::style;

fn print_help() {
    println!(
        "{}\n \
        \t{}\n \
        \t\tActivates the bot and connects to Discord.\n \
        \t{}\n \
        \t\tDisconnects from Discord, does not exit.\n \
        \t{}\n \
        \t\tDisconnects from Discord and exits.\n \
        \t{}\n \
        \t\tShow this help message.\n",
    style("Commands:").green(),
    style("activate").cyan(),
    style("deactivate").cyan(),
    style("quit").cyan(),
    style("help").cyan());
}

#[tokio::main]
async fn main() {
    let (cli_tx, mut cli_rx) = tokio::sync::mpsc::channel::<String>(1);
    let (main_tx, mut main_rx) = tokio::sync::mpsc::channel::<bool>(1);

    let cli_handle = thread::spawn(move || {
        // CLI input loop.
        loop {
            if main_rx.blocking_recv().unwrap() == false {
                break;
            }
            let input: String = Input::<String>::new()
                .with_prompt(">")
                .default("help".into())
                .interact_text()
                .unwrap();
            cli_tx.blocking_send(input).unwrap();
        }
    });

    // Kick off the CLI.
    main_tx.send(true).await.unwrap();

    // Main event loop.
    loop {
        tokio::select! {
            // Input from the CLI.
            Some(msg) = cli_rx.recv() => {
                let mut rsp = true;
                println!("Received: {}", style(&msg).cyan());
                match msg.as_str() {
                    "help" | "h" => print_help(),
                    "quit" | "q" => rsp = false,
                    _ => ()
                }
                main_tx.send(rsp).await.unwrap();
            }
            // CLI channel dropped, time to exit.
            else => {
                println!("Exiting...");
                break
            }
        }
    }
    cli_handle.join().unwrap();
}

// let listener = TcpListener::bind("localhost:8080").await.unwrap();
// let (tx, _rx) = broadcast::channel(10);
// let mut client_count: usize = 0;

// loop {
//     let (mut socket, addr) = listener.accept().await.unwrap();
//     let tx = tx.clone();
//     let mut rx = tx.subscribe();
//     let id = client_count;
//     client_count += 1;

//     tokio::spawn(async move {
//         let (reader, mut writer) = socket.split();
//         let mut reader = BufReader::new(reader);
//         let mut line = String::new();
//         loop {
//             tokio::select! {
//                 result = reader.read_line(&mut line) => {
//                     let num_bytes = result.unwrap();
//                     if num_bytes == 0 {
//                         println!("client {} disconnected", id);
//                         break;
//                     }
//                     println!("client {} bytes_read: {}", id, num_bytes);
//                     let msg = format!("Client {}: {}", id, line);
//                     tx.send((msg, addr)).unwrap();
//                     line.clear();
//                 }
//                 result = rx.recv() => {
//                     let (msg, sender_addr) = result.unwrap();
//                     if addr != sender_addr {
//                         writer.write_all(msg.as_bytes()).await.unwrap();
//                     }
//                 }
//             }
//         }
//     });
