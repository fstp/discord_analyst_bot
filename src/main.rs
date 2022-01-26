//#![feature(unboxed_closures)]

use dialoguer::Input;
use std::thread;
use console::style;

fn print_help() {
    let spring_green = console::Color::Color256(29);
    println!(
        "{}\n \
        \t{}\n \
        \t\tActivates the bot and connects to Discord.\n\n \
        \t{}\n \
        \t\tDisconnects from Discord, does not exit.\n\n \
        \t{} {}\n \
        \t\tAdds a new source channel. The <tag> refers to the indicator to specify a channel.\n \
        \t\tEg. If an analyst alerts stocks and options, they may designate 1 as the stocks channel tag\n \
        \t\tand 2 as the options channel tag, or they may use the words \"stocks\" and \"options\" if they want.\n\n \
        \t{} {}\n \
        \t\tRemoves a source channel. The <tag> is optional. Excluding this tag would erase the whole channel\n \
        \t\tfrom the bot's db. Including the <#> would only remove that instance\n \
        \t\t(in the event that someone has multiple tags assigned to the same source channel).\n\n \
        \t{} {}\n \
        \t\tAdds a new target channel to the server in which the command is typed..\n \
        \t\tThe <tag> refers to the numerical tag and ties to the corresponding source tag.\n \
        \t\tOne target channel can link to more than one source channel.\n\n \
        \t{}\n \
        \t\tLists all servers that the bot is installed in, including server ID and server name,\n \
        \t\twith clickable links to the target channel(s) that the bot is in. If the bot does not\n \
        \t\thave any target channels in that server then it would not no target channels.\n\n \
        \t{} {}\n \
        \t{} {}\n \
        \t\tBanlist disables bot alerts in all target channels in the server. Once a week at 9:00am EST\n \
        \t\ton Monday sends a message with a RED embed saying:\n \
        \t\t\"This bot has been disabled in this server. Please contact @k-sauce#9999 to re-enable the bot.\"\n\n \
        \t{} {}\n \
        \t\tAdds a role @mention to the alerts in the specified target channel. The #tag is used to link\n \
        \t\tto the source channel. There can be more than one tag per target channel. The #tag is optional.\n \
        \t\tIf no #tag is used, the role will apply to all the alerts in the target channel regardless of\n \
        \t\tthe source channel.\n\n \
        \t{} {}\n \
        \t\tHere the #tag and @role are optional. If no #tag or @role are mentioned,\n \
        \t\t{} of the @mentions in the target channel are removed.. If there is a\n \
        \t\t#tag but no @role, all the @mentions relating to that #tag are removed.\n \
        \t\tIf there is a @role but no #tag, all the @mentions of that @role are removed.\n\n \
        \t{} {}\n \
        \t\tRecalls the last [#] messages sent from the source channel that the messages\n \
        \t\tare sent from. Defaults to 1 if no # is provided.\n\n \
        \t{}\n \
        \t\tDisconnects from Discord and exits.\n\n \
        \t{}\n \
        \t\tShow this help message.\n",
    style("Commands:").fg(spring_green),
    style("activate").cyan(),
    style("deactivate").cyan(),
    style("source+").cyan(), style("#channel <tag>").green(),
    style("source-").cyan(), style("#channel <tag>").green(),
    style("target+").cyan(), style("#channel <tag>").green(),
    style("serverlist").cyan(),
    style("serverbanlist+").cyan(), style("<Server ID>").green(),
    style("serverbanlist-").cyan(), style("<Server ID>").green(),
    style("mention+").cyan(), style("#channel [<tag>] @role").green(),
    style("mention-").cyan(), style("#channel [<tag>] [@role]").green(), style("ALL").red(),
    style("recall").cyan(), style("<#>").green(),
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
