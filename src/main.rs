#![feature(hash_drain_filter)]
#![feature(io_error_other)]

use anyhow::{anyhow, bail, Context, Error, Result};
use console::style;
use dialoguer::Input;
use futures::TryFutureExt;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serenity::{
    async_trait,
    client::Context as ClientContext, // Alias to avoid name collision with anyhow::Context
    model::{
        channel::{ChannelType, Embed, GuildChannel, Message, PartialChannel},
        gateway::Ready,
        id::{ChannelId, GuildId, UserId, WebhookId},
        interactions::{
            application_command::{
                ApplicationCommandInteraction, ApplicationCommandInteractionDataOption,
                ApplicationCommandInteractionDataOptionValue, ApplicationCommandOptionType,
            },
            autocomplete::AutocompleteInteraction,
            Interaction, InteractionResponseType,
        },
    },
    prelude::*,
    utils::Color,
};
use sqlx::SqlitePool;
use std::{
    cmp,
    collections::{HashMap, HashSet},
    fmt::Display,
    sync::Arc,
    thread,
};
use sublime_fuzzy::best_match;
use tokio::fs;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

struct CommandResponse {
    title: String,
    msg: String,
}

struct AutocompleteResponse {
    options: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
struct Server {
    name: String,
    id: GuildId,
    channels: HashMap<String, ChannelId>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
struct SourceChannel {
    name: String,
    channel_tag: String,
    server_tag: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
struct TargetChannel {
    name: String,
    source_tag: String,
    server_tag: String,
    channel_id: ChannelId,
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct Data {
    source_channels: HashMap<ChannelId, SourceChannel>,
    target_channels: HashSet<TargetChannel>,
    channel_mapping: HashMap<String, HashSet<ChannelId>>,
    server_mapping: HashMap<String, Server>,
    next_server_tag: usize,
}

const SPRING_GREEN: console::Color = console::Color::Color256(29);

fn print_help() {
    println!(
        "{}\n \
        \t{} {}\n \
        \t\tAdds a new server where the bot can operate. The <server-tag> will refer to this server\n \
        \t\twhen creating source/target channels so that the bot can connect one source with a target\n \
        \t\ton another server.\n \
        \t{} {}\n \
        \t\tAdds a new source channel. The <channel-tag> refers to the indicator to specify a channel.\n \
        \t\tEg. If an analyst alerts stocks and options, they may designate 1 as the stocks channel tag\n \
        \t\tand 2 as the options channel tag, or they may use the words \"stocks\" and \"options\" if they want.\n\n \
        \t{} {}\n \
        \t\tRemoves a source channel. The <channel-tag> is optional. Excluding this tag would erase the\n \
        \t\twhole channel from the bot's db. Including the <#> would only remove that instance\n \
        \t\t(in the event that someone has multiple tags assigned to the same source channel).\n\n \
        \t{} {}\n \
        \t\tAdds a new target channel to the server in which the command is typed..\n \
        \t\tThe <channel-tag> refers to the numerical tag and ties to the corresponding source tag.\n \
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
        \t\tStore the current state into \"data.json\"\n \
        \t\t{} This will override any existing state that is already stored in the file.\n\n \
        \t{}\n \
        \t\tLoad state from \"data.json\".\n \
        \t\t{} This will override any existing state that has not yet been saved.\n\n \
        \t{}\n \
        \t\tPrint a table with the currently connected source/target channels.\n\n \
        \t{}\n \
        \t\tShow this help message.\n",
    style("Commands:").fg(SPRING_GREEN),
    style("server+").cyan(), style("<server-tag> name").green(),
    style("source+").cyan(), style("<server-tag> #channel <channel-tag>").green(),
    style("source-").cyan(), style("#channel <channel-tag>").green(),
    style("target+").cyan(), style("<server-tag> #channel <channel-tag>").green(),
    style("serverlist").cyan(),
    style("serverbanlist+").cyan(), style("<Server ID>").green(),
    style("serverbanlist-").cyan(), style("<Server ID>").green(),
    style("mention+").cyan(), style("#channel [<tag>] @role").green(),
    style("mention-").cyan(), style("#channel [<tag>] [@role]").green(), style("ALL").red(),
    style("recall").cyan(), style("<#>").green(),
    style("quit").cyan(),
    style("save").cyan(), style("Warning:").red(),
    style("load").cyan(), style("Warning:").red(),
    style("status").cyan(),
    style("help").cyan());
}

fn add_mapping(source_tag: String, target_channel_id: ChannelId, data: &mut Data) {
    // First check if the source tag already exists in the mapping.
    if data.channel_mapping.contains_key(&source_tag) {
        let target_channel_ids = data.channel_mapping.get_mut(&source_tag).unwrap();
        target_channel_ids.insert(target_channel_id);
    } else {
        // This is the first occurence of source tag so create a new association.
        data.channel_mapping
            .insert(source_tag, HashSet::from([target_channel_id]));
    }
}

fn validate_server_tag<'a>(server_tag: &String, data: &'a Data) -> Option<&'a Server> {
    return match data.server_mapping.get(server_tag) {
        Some(server) => Some(server),
        None => {
            println!(
                "{}\nNo server with the tag {} was found",
                style("Error:").red(),
                style(server_tag).cyan(),
            );
            None
        }
    };
}

fn validate_channel_name(channel_name: &String, server: &Server) -> Option<ChannelId> {
    for (name, id) in &server.channels {
        if name == channel_name {
            return Some(*id);
        }
    }
    // If we get here then the channel was not found, error.
    println!(
        "{}\nNo channel with the name {} found in the server {}",
        style("Error:").red(),
        style(channel_name).cyan(),
        style(&server.name).cyan(),
    );
    return None;
}

async fn handle_input(msg: String, data: Arc<Mutex<Data>>) -> bool {
    let mut rsp = true;
    let parts: Vec<&str> = msg.split_whitespace().collect();
    let mut data = data.lock().await;
    match parts[0] {
        "help" | "h" => print_help(),
        "quit" | "q" => rsp = false,
        "save" | "s" => {
            let serialized = match serde_json::to_string_pretty(&*data) {
                Ok(serialized) => serialized,
                Err(why) => {
                    println!(
                        "{}\nFailed to serialize the data (reason: {})",
                        style("Error").red(),
                        style(why).cyan()
                    );
                    return rsp;
                }
            };
            let mut file = match File::create("data.json").await {
                Ok(file) => file,
                Err(why) => {
                    println!(
                        "{}\nFailed to create the \"data.json\" file (reason: {}) \
                        \nAre you sure that you have access rights to create/write files \
                        \nin the bot directory?",
                        style("Error").red(),
                        style(why).cyan()
                    );
                    return rsp;
                }
            };
            match file.write_all(serialized.as_bytes()).await {
                Ok(_) => {
                    println!("{}:\n{}", style("Serialized").cyan(), serialized);
                }
                Err(why) => {
                    println!(
                        "{}\nFailed to write to the \"data.json\" file (reason: {}) \
                        \nAre you sure that you have access rights to create/write files \
                        \nin the bot directory?",
                        style("Error").red(),
                        style(why).cyan()
                    );
                    return rsp;
                }
            };
        }
        "load" | "l" => match fs::read_to_string("data.json").await {
            Ok(json) => match serde_json::from_str(&json) {
                Ok(deserialized) => {
                    println!("{}:\n{:#?}", style("Deserialized").cyan(), deserialized);
                    *data = deserialized;
                }
                Err(why) => {
                    println!(
                        "{}\nFailed to deserialize the data from \"data.json\" file (reason: {}) \
                            \nPerhaps something in the JSON structure is incorrect.",
                        style("Error").red(),
                        style(why).cyan()
                    );
                    return rsp;
                }
            },
            Err(why) => {
                println!(
                    "{}\nFailed to read the file \"data.json\" (reason: {}) \
                    \nAre you sure it exists in the same directory as the bot?",
                    style("Error:").red(),
                    style(why).cyan(),
                )
            }
        },
        "debug_dump" | "dd" => {
            println!("{:#?}", data);
        }
        // "server+" if parts.len() > 2 => {
        //     let server_tag = parts[1].to_owned();
        //     let server_name = parts[2..].join(" ");
        //     println!("{}", server_name);
        //     data.server_mapping.insert(server_tag, server_name);
        // }
        "source+" if parts.len() == 4 => {
            let server_tag = parts[1].to_owned();
            let name = parts[2].to_owned();
            let channel_tag = parts[3].to_owned();

            let server = match validate_server_tag(&server_tag, &data) {
                Some(server) => server,
                None => return rsp,
            };

            let channel_id = match validate_channel_name(&name, server) {
                Some(channel_id) => channel_id,
                None => return rsp,
            };

            // Reconnect any orphan target channels with this source channel tag.
            let mut mappings: Vec<(String, ChannelId)> = Vec::default();
            for ch in &data.target_channels {
                if &ch.source_tag == &channel_tag {
                    mappings.push((channel_tag.clone(), ch.channel_id));
                }
            }
            for m in mappings {
                add_mapping(m.0, m.1, &mut *data);
            }

            let source_channel = SourceChannel {
                server_tag: server_tag,
                name: name,
                channel_tag: channel_tag,
            };
            data.source_channels.insert(channel_id, source_channel);
        }
        "target+" if parts.len() == 4 => {
            let server_tag = parts[1].to_owned();
            let name = parts[2].to_owned();
            let source_tag = parts[3].to_owned();

            let server = match validate_server_tag(&server_tag, &data) {
                Some(server) => server,
                None => return rsp,
            };

            let channel_id = match validate_channel_name(&name, server) {
                Some(channel_id) => channel_id,
                None => return rsp,
            };

            // Make sure we actually know about the source channel (valid source tag).
            let mut found = false;
            for (_ch_id, ch) in &data.source_channels {
                if &ch.channel_tag == &source_tag {
                    add_mapping(source_tag.clone(), channel_id, &mut *data);
                    found = true;
                    break;
                }
            }

            if found == false {
                // No source channel found, error.
                println!(
                    "{}\nNo source channel found with the tag {}",
                    style("Error:").red(),
                    style(source_tag).cyan(),
                );
                return rsp;
            }

            let target_channel = TargetChannel {
                server_tag: server_tag,
                name: name,
                source_tag: source_tag,
                channel_id: channel_id,
            };
            data.target_channels.insert(target_channel);
        }
        // <channel-tag> is a parameter.
        "source-" if parts.len() == 3 => {
            let name = parts[2].to_owned();
            let channel_tag = parts[3].to_owned();

            let mut iter = data.source_channels.drain_filter(|_ch_id, ch| {
                return (&ch.channel_tag == &channel_tag) && (&ch.name == &name);
            });

            if iter.next().is_some() == false {
                // No channel was found/removed, error.
                println!(
                    "{}\nNo source channel with name {} was found",
                    style("Error:").red(),
                    style(&name).cyan(),
                );
            }
        }
        // "source-" if parts.len() == 2 => {
        //     // Not specifying tag so remove all instances.
        //     let name = parts[1].to_owned();
        //     let drained: Vec<(String, SourceChannel)> = data
        //         .source_channels
        //         .drain_filter(|_tag, ch| ch.name == name)
        //         .collect();
        //     for ch in drained {
        //         data.tag_mapping.remove(&ch.0);
        //     }
        // }
        // "status" => {
        //     let mut table = Table::new();
        //     let header = vec![
        //         Cell::new("Source Channel").add_attribute(Attribute::Bold),
        //         Cell::new("Target Channel(s)").add_attribute(Attribute::Bold),
        //     ];
        //     let mut rows: Vec<Vec<Cell>> = Vec::default();
        //     for (tag, ch) in &data.source_channels {
        //         // Only include source channel in the table if it has
        //         // any targets mapped to it.
        //         if data.channel_mapping.contains_key(tag) {
        //             let targets = data.channel_mapping.get(tag).unwrap().iter().format("\n");
        //             rows.push(vec![
        //                 Cell::new(format!("{} [{}]", ch.name, ch.channel_tag)).fg(Color::Cyan),
        //                 Cell::new(format!("{}", targets)).fg(Color::Cyan),
        //             ]);
        //         }
        //     }
        //     table
        //         .load_preset(UTF8_FULL)
        //         .set_content_arrangement(ContentArrangement::Dynamic)
        //         .set_table_width(80)
        //         .set_header(header);
        //     for row in rows {
        //         table.add_row(row);
        //     }
        //     println!("{table}");
        // }
        _ => {
            println!(
                "{} Unrecognized command\n{:#?}",
                style("Error:").red(),
                parts
            );
        }
    }
    return rsp;
}

async fn create_server_mapping(db: &SqlitePool, ctx: &ClientContext, guilds: &Vec<GuildId>) {
    for id in guilds {
        let guild_id = id.0 as i64;
        let name = id.name(&ctx).await.unwrap();
        sqlx::query!(
            "INSERT OR REPLACE INTO Guilds (id, name) VALUES (?, ?)",
            guild_id,
            name,
        )
        .execute(db)
        .await
        .unwrap();

        let channels: Vec<(ChannelId, GuildChannel)> =
            id.channels(&ctx).await.unwrap().into_iter().collect();
        for (ch_id, ch) in channels {
            if ch.kind == ChannelType::Text {
                let channel_id = ch_id.0 as i64;
                let name = format!("#{}", ch.name);
                sqlx::query!(
                    "INSERT OR REPLACE INTO Channels (id, name, guild) VALUES (?, ?, ?)",
                    channel_id,
                    name,
                    guild_id,
                )
                .execute(db)
                .await
                .unwrap();
            }
        }
    }
}

async fn get_guild_names(db: &SqlitePool) -> Result<Vec<String>> {
    sqlx::query!("SELECT Guilds.name FROM Guilds")
        .fetch_all(db)
        .and_then(|result| async { Ok(result.into_iter().map(|record| record.name).collect()) })
        .await
        .map_err(|e| anyhow!(e).context("Failed to retrieve guild names from the database"))
}

async fn get_guild_ids(db: &SqlitePool) -> Vec<GuildId> {
    sqlx::query!("SELECT Guilds.id FROM Guilds")
        .fetch_all(db)
        .await
        .unwrap()
        .into_iter()
        .map(|record| GuildId(record.id as u64))
        .collect()
}

async fn get_channel_names(server_name: &String, db: &SqlitePool) -> Result<Vec<String>> {
    sqlx::query!(
        "
        SELECT Channels.name\n\
        FROM Channels\n\
        JOIN Guilds\n\
        ON Guilds.name = ? AND Channels.guild = Guilds.id",
        server_name
    )
    .fetch_all(db)
    .and_then(|records| async { Ok(records.into_iter().map(|record| record.name).collect()) })
    .await
    .map_err(|e| anyhow!(e).context("Failed to retrieve channel names from database"))
}

struct Handler {
    data: Arc<Mutex<Data>>,
    db: SqlitePool,
    cache_rdy_tx: tokio::sync::mpsc::Sender<bool>,
}

#[async_trait]
impl EventHandler for Handler {
    // Set a handler for the `message` event - so that whenever a new message
    // is received - the closure (or function) passed will be called.
    //
    // Event handlers are dispatched through a threadpool, and so multiple
    // events can be dispatched simultaneously.
    //async fn message(&self, _ctx: Context, _msg: Message) {
    // Sending a message can fail, due to a network error, an
    // authentication error, or lack of permissions to post in the
    // channel, so log to stdout when some error happens, with a
    // description of it.
    // let user = msg.author;
    // if user.bot == false {
    //     let guild_name = msg.guild_id.unwrap().name(ctx.cache).await.unwrap();
    //     let str_msg = format!(
    //         "{} said in {} ({}): {}",
    //         user.mention(),
    //         msg.channel_id.mention(),
    //         guild_name,
    //         msg.content,
    //     );
    //     if let Err(why) = msg.channel_id.say(&ctx.http, str_msg).await {
    //         println!("Error sending message: {:?}", why);
    //     }
    // }
    //}

    // Set a handler to be called on the `ready` event. This is called when a
    // shard is booted, and a READY payload is sent by Discord. This payload
    // contains data like the current user's guild Ids, current user data,
    // private channels, and more.
    async fn ready(&self, ctx: ClientContext, ready: Ready) {
        println!("{} is connected to Discord", ready.user.name);
        let guild_ids = get_guild_ids(&self.db).await;
        for id in guild_ids {
            let result = GuildId::set_application_commands(&id, &ctx.http, |commands| {
                commands
                    .create_application_command(|command| {
                        command
                            .name("connect")
                            .description("Connect a source channel to a target channel")
                            .create_option(|option| {
                                option
                                    .name("source")
                                    .description("Source channel")
                                    .kind(ApplicationCommandOptionType::Channel)
                                    .required(true)
                                //.set_autocomplete(true)
                            })
                            .create_option(|option| {
                                option
                                    .name("target_server")
                                    .description("Target server")
                                    .kind(ApplicationCommandOptionType::String)
                                    .required(true)
                                    .set_autocomplete(true)
                            })
                            .create_option(|option| {
                                option
                                    .name("target_channel")
                                    .description("Target channel")
                                    .kind(ApplicationCommandOptionType::String)
                                    .required(true)
                                    .set_autocomplete(true)
                            })
                    })
                    .create_application_command(|command| {
                        command
                            .name("disconnect")
                            .description("Disconnect one target channel from a source channel")
                            .create_option(|option| {
                                option
                                    .name("source")
                                    .description("Source channel")
                                    .kind(ApplicationCommandOptionType::Channel)
                                    .required(true)
                                //.set_autocomplete(true)
                            })
                            .create_option(|option| {
                                option
                                    .name("target_channel")
                                    .description("Target channel")
                                    .kind(ApplicationCommandOptionType::String)
                                    .required(true)
                                    .set_autocomplete(true)
                            })
                    })
                    .create_application_command(|command| {
                        command
                            .name("disconnect-all")
                            .description("Disconnect all target channels from a source channel")
                            .create_option(|option| {
                                option
                                    .name("source")
                                    .description("Source channel")
                                    .kind(ApplicationCommandOptionType::Channel)
                                    .required(true)
                                //.set_autocomplete(true)
                            })
                    })
                    .create_application_command(|command| {
                        command
                            .name("list-connections")
                            .description("List all the active connections for a particular server")
                    })
                    .create_application_command(|command| {
                        command
                            .name("wipe")
                            .description(
                                "[WARNING] Will remove ALL connections to/from the selected server",
                            )
                            .create_option(|option| {
                                option
                                    .name("server")
                                    .description("Server name")
                                    .kind(ApplicationCommandOptionType::String)
                                    .required(true)
                                    .set_autocomplete(true)
                            })
                    })
            })
            .await;
            let guild_name = id.name(&ctx).await.unwrap();
            match result {
                Ok(_) => println!("Successfully installed slash commands in {guild_name}"),
                Err(why) => println!("Failed to install slash commands in {guild_name}: {why}"),
            }
        }
    }

    async fn cache_ready(&self, ctx: ClientContext, guilds: Vec<GuildId>) {
        create_server_mapping(&self.db, &ctx, &guilds).await;
        self.cache_rdy_tx
            .send(true)
            .await
            .expect("Failed to send cache ready");
    }

    async fn message(&self, ctx: ClientContext, msg: Message) {
        match handle_message(&self.db, &ctx, &msg).await {
            Ok(_) => (),
            Err(e) => {
                println!("{:?}", e)
            }
        }
    }

    async fn interaction_create(&self, ctx: ClientContext, interaction: Interaction) {
        match interaction {
            Interaction::ApplicationCommand(command) => {
                handle_application_command(&self.db, &command, &ctx).await
            }
            Interaction::Autocomplete(autocomplete) => {
                handle_autocomplete(&self.db, &autocomplete, &ctx).await
            }
            _ => println!("Received unknown interaction:\n{:#?}", interaction),
        }
    }
}

async fn handle_message(db: &SqlitePool, ctx: &ClientContext, msg: &Message) -> Result<()> {
    if msg.author.bot == true {
        return Ok(());
    }

    let source = msg.channel_id.0 as i64;
    let user = msg.author.id.0 as i64;
    let webhook_ids: Vec<WebhookId> = sqlx::query!(
        "
        SELECT webhook as \"webhook_id: i64\"\n\
        FROM Connections\n\
        WHERE Connections.source = ? AND Connections.user = ?
        ",
        source,
        user,
    )
    .fetch_all(db)
    .and_then(|rows| async move {
        Ok(rows
            .into_iter()
            .map(|row| WebhookId(row.webhook_id as u64))
            .collect())
    })
    .map_err(|e| Error::new(e).context("Failed to retrieve webhook ids from database"))
    .await?;

    for id in webhook_ids {
        let webhook = id.to_webhook(&ctx).await?;
        webhook
            .execute(&ctx, false, |w| {
                //w.content(&msg.content);
                let embed = Embed::fake(|e| {
                    e /*.author(|a| a.name(username).url(user_url).icon_url(icon_url))*/
                        .color(Color::BLUE)
                        .description(&msg.content)
                });
                w.embeds(vec![embed])
            })
            .await
            .context(format!("Failed to execute webhook:\n{:#?}", webhook))?;
    }

    Ok(())
}

async fn send_empty_response(autocomplete: &AutocompleteInteraction, ctx: &ClientContext) {
    autocomplete
        .create_autocomplete_response(&ctx, move |rsp| rsp)
        .await
        .unwrap()
}

async fn disconnect_target_channel_autocomplete(
    db: &SqlitePool,
    source_channel: &ApplicationCommandInteractionDataOption,
    target_channel: &ApplicationCommandInteractionDataOption,
) -> Result<AutocompleteResponse> {
    let target_channel = match &target_channel.value {
        Some(serde_json::Value::String(input)) => input.clone(),
        Some(val) => bail!("Expected option to be of type string:\n{:#?}", val),
        None => bail!("Did not find option \"target_channel\""),
    };

    let source_channel: i64 = match &source_channel.value {
        Some(serde_json::Value::String(input)) => input
            .parse()
            .context("Failed to parse \"source_channel\"")?,
        Some(val) => bail!("Expected option to be of type string:\n{:#?}", val),
        None => bail!("Did not find option \"target_channel\""),
    };

    let channels: Vec<String> = sqlx::query!(
        "
        SELECT\n\
        Guilds.name as guild_name,\n\
        Channels.name as channel_name\n\
        FROM Channels\n\
        JOIN Connections\n\
        ON Channels.id = Connections.target\n\
        JOIN Guilds\n\
        ON Channels.guild = Guilds.id\n\
        WHERE Connections.source = ?\n\
        ORDER BY Guilds.name
        ",
        source_channel
    )
    .fetch_all(db)
    .and_then(|rows| async move {
        Ok(rows
            .into_iter()
            .map(|row| format!("[{}] {}", row.guild_name, row.channel_name))
            .collect())
    })
    .await
    .context("Failed to retrieve target channel names from the database")?;

    if channels.is_empty() {
        bail!("No target channels found")
    }

    // Matching score, lower score is a better match.
    let mut matching: Vec<(isize, String)> = channels
        .into_iter()
        .map(|s| {
            let score = match best_match(target_channel.as_str(), s.as_str()) {
                Some(m) => (100 - m.score(), s),
                None => (100, s),
            };
            score
        })
        .collect();

    matching.sort();
    matching.drain(cmp::min(25, matching.len())..);

    Ok(AutocompleteResponse {
        options: matching.into_iter().map(|(_score, name)| name).collect(),
    })
}

async fn connect_target_channel_autocomplete(
    db: &SqlitePool,
    server_name: &String,
    opt: &ApplicationCommandInteractionDataOption,
) -> Result<AutocompleteResponse> {
    if server_name.trim().is_empty() {
        bail!("No server name");
    }

    let channel_name = match &opt.value {
        Some(serde_json::Value::String(input)) => input.clone(),
        _ => bail!("Expected option to be of type string:\n{:#?}", opt.value),
    };

    let channels = get_channel_names(server_name, db).await?;

    // Matching score, lower score is a better match.
    let mut matching: Vec<(isize, String)> = channels
        .into_iter()
        .map(|s| {
            let score = match best_match(channel_name.as_str(), s.as_str()) {
                Some(m) => (100 - m.score(), s),
                None => (100, s),
            };
            score
        })
        .collect();

    if matching.is_empty() {
        bail!("No matching channels");
    }

    matching.sort();
    matching.drain(cmp::min(25, matching.len())..);

    Ok(AutocompleteResponse {
        options: matching.into_iter().map(|(_score, name)| name).collect(),
    })
}

async fn connect_target_server_autocomplete(
    db: &SqlitePool,
    server_name: &String,
) -> Result<AutocompleteResponse> {
    let servers = get_guild_names(db).await?;

    // Matching score, lower score is a better match.
    let mut matching: Vec<(isize, String)> = servers
        .into_iter()
        .map(|s| {
            let score = match best_match(server_name.as_str(), s.as_str()) {
                Some(m) => (100 - m.score(), s),
                None => (100, s),
            };
            score
        })
        .collect();

    if matching.is_empty() {
        bail!("No guilds found");
    }

    matching.sort();
    matching.drain(cmp::min(25, matching.len())..);

    Ok(AutocompleteResponse {
        options: matching.into_iter().map(|(_score, name)| name).collect(),
    })
}

fn find_param<'a>(
    name: &str,
    autocomplete: &'a AutocompleteInteraction,
) -> Result<&'a ApplicationCommandInteractionDataOption> {
    autocomplete
        .data
        .options
        .iter()
        .find(|opt| opt.name == name)
        .ok_or(anyhow!("Did not find autocomplete parameter: {name}"))
}

async fn handle_autocomplete(
    db: &SqlitePool,
    autocomplete: &AutocompleteInteraction,
    ctx: &ClientContext,
) {
    let result: Result<AutocompleteResponse> = match autocomplete.data.name.as_str() {
        "connect" => handle_connect_autocomplete(db, autocomplete).await,
        "disconnect" => handle_disconnect_autocomplete(db, autocomplete).await,
        "wipe" => handle_wipe_autocomplete(db, autocomplete).await,
        s => Err(anyhow!("Unhandled autocomplete:\n{s}")),
    };
    match result {
        Ok(rsp) => autocomplete
            .create_autocomplete_response(&ctx, move |c| {
                for name in rsp.options {
                    c.add_string_choice(name.as_str(), name.as_str());
                }
                c
            })
            .await
            .unwrap(),
        Err(e) => {
            println!("{:?}", e);
            send_empty_response(autocomplete, ctx).await;
        }
    }
}

async fn handle_wipe_autocomplete(
    db: &SqlitePool,
    autocomplete: &AutocompleteInteraction,
) -> Result<AutocompleteResponse> {
    let param_target_server = find_param("server", &autocomplete)?;

    let server_name = match &param_target_server.value {
        Some(serde_json::Value::String(input)) => input.clone(),
        Some(val) => bail!("Unexpected parameter type (expected string):\n{:#?}", val),
        None => bail!("No parameter value found"),
    };

    connect_target_server_autocomplete(db, &server_name).await
}

async fn handle_connect_autocomplete(
    db: &SqlitePool,
    autocomplete: &AutocompleteInteraction,
) -> Result<AutocompleteResponse> {
    let param_target_server = find_param("target_server", &autocomplete)?;
    let param_target_channel = find_param("target_channel", &autocomplete);

    let server_name = match &param_target_server.value {
        Some(serde_json::Value::String(input)) => input.clone(),
        Some(val) => bail!("Unexpected parameter type (expected string):\n{:#?}", val),
        None => bail!("No parameter value found"),
    };

    if param_target_server.focused {
        connect_target_server_autocomplete(db, &server_name).await
    } else if param_target_channel.is_ok() {
        let param_target_channel = param_target_channel.unwrap();
        if param_target_channel.focused {
            connect_target_channel_autocomplete(db, &server_name, &param_target_channel).await
        } else {
            bail!("Target channel not focused")
        }
    } else {
        bail!("Invalid parameter focus")
    }
}

async fn handle_disconnect_autocomplete(
    db: &SqlitePool,
    autocomplete: &AutocompleteInteraction,
) -> Result<AutocompleteResponse> {
    let param_source_channel = find_param("source", &autocomplete)?;
    let param_target_channel = find_param("target_channel", &autocomplete)?;

    if param_target_channel.focused {
        disconnect_target_channel_autocomplete(db, &param_source_channel, &param_target_channel)
            .await
    } else {
        bail!("Target channel not focused")
    }
}

async fn ok_command_response(
    title: &impl Display,
    msg: &impl Display,
    command: &ApplicationCommandInteraction,
    ctx: &ClientContext,
) {
    if let Err(why) = command
        .create_interaction_response(&ctx.http, |response| {
            response
                .kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|message| {
                    message.create_embed(|e| {
                        e /*.author(|a| a.name(username).url(user_url).icon_url(icon_url))*/
                            .color(Color::DARK_GREEN)
                            .title(title)
                            .description(msg)
                    })
                })
        })
        .await
    {
        println!("Cannot respond to slash command: {why}");
    }
}

async fn error_command_response(
    msg: &impl Display,
    command: &ApplicationCommandInteraction,
    ctx: &ClientContext,
) {
    if let Err(why) = command
        .create_interaction_response(&ctx.http, |response| {
            response
                .kind(InteractionResponseType::ChannelMessageWithSource)
                .interaction_response_data(|message| {
                    message.create_embed(|e| e.color(Color::RED).title("Error").description(&msg))
                })
        })
        .await
    {
        println!(
            "Cannot respond to slash command: {}\nError message: {}",
            why, msg
        );
    }
}

fn get_channel_opt<'a>(
    name: &str,
    options: &'a Vec<ApplicationCommandInteractionDataOption>,
) -> Result<&'a PartialChannel> {
    options
        .iter()
        .find(|&opt| opt.name == name)
        .and_then(|op| {
            op.resolved.as_ref().and_then(|ch| match ch {
                ApplicationCommandInteractionDataOptionValue::Channel(ch) => Some(ch),
                _ => None,
            })
        })
        .ok_or(anyhow!("Failed to retrieve channel option: \"{}\"", name))
}

fn get_string_opt<'a>(
    name: &str,
    options: &'a Vec<ApplicationCommandInteractionDataOption>,
) -> Result<&'a String> {
    options
        .iter()
        .find(|&opt| opt.name == name)
        .and_then(|op| {
            op.resolved.as_ref().and_then(|ch| match ch {
                ApplicationCommandInteractionDataOptionValue::String(s) => Some(s),
                _ => None,
            })
        })
        .ok_or(anyhow!("Failed to retrieve string option: \"{}\"", name))
}

async fn name_to_ids(
    db: &SqlitePool,
    server_name: &String,
    channel_name: &String,
) -> Result<(GuildId, ChannelId)> {
    sqlx::query!(
        "
        SELECT\n\
        Guilds.name as guild_name,\n\
        Guilds.id as \"guild_id: i64\",\n\
        Channels.name as channel_name,\n\
        Channels.id as \"channel_id: i64\"\n\
        FROM Channels\n\
        JOIN Guilds\n\
        ON Channels.guild = Guilds.id\n\
        WHERE guild_name = ? AND channel_name = ?
        ",
        server_name,
        channel_name,
    )
    .fetch_one(db)
    .and_then(|row| async move {
        Ok((
            GuildId(row.guild_id as u64),
            ChannelId(row.channel_id as u64),
        ))
    })
    .await
    .map_err(|e| Error::new(e).context("Failed to convert server/channel names to ids"))
}

async fn get_webhook_id(
    db: &SqlitePool,
    user_id: &UserId,
    target_channel_id: &ChannelId,
) -> Result<Option<WebhookId>> {
    let user = user_id.0 as i64;
    let target = target_channel_id.0 as i64;
    sqlx::query!(
        "
        SELECT id as \"webhook_id: i64\"\n\
        FROM Webhooks\n\
        WHERE Webhooks.user = ? AND Webhooks.target = ?
        ",
        user,
        target,
    )
    .fetch_optional(db)
    .and_then(|row| async move { Ok(row.map(|row| WebhookId(row.webhook_id as u64))) })
    .await
    .map_err(|e| Error::new(e).context("Failed to retrieve webhook from database"))
}

async fn maybe_add_webhook(
    db: &SqlitePool,
    user_id: &UserId,
    target_channel_id: &ChannelId,
    ctx: &ClientContext,
) -> Result<WebhookId> {
    match get_webhook_id(db, user_id, target_channel_id).await? {
        Some(id) => return Ok(id),
        None => (),
    }
    let username = user_id.to_user(&ctx).await?.name.clone();
    let target_channel = target_channel_id
        .to_channel(&ctx)
        .await?
        .guild()
        .ok_or(anyhow!("Failed to get the guild channel"))?;

    let webhook_id = target_channel.create_webhook(&ctx, username).await?.id;
    let id = webhook_id.0 as i64;
    let user = user_id.0 as i64;
    let target = target_channel_id.0 as i64;
    let result = sqlx::query!(
        "INSERT OR REPLACE INTO Webhooks (id, target, user) VALUES (?, ?, ?)",
        id,
        target,
        user,
    )
    .execute(db)
    .await;
    match result {
        Ok(_) => Ok(webhook_id),
        Err(e) => Err(Error::new(e).context("Failed to insert webhook into the database")),
    }
}

async fn connection_exists(
    db: &SqlitePool,
    source_channel_id: &ChannelId,
    target_channel_id: &ChannelId,
    webhook_id: &WebhookId,
) -> Result<bool, sqlx::Error> {
    let source = source_channel_id.0 as i64;
    let target = target_channel_id.0 as i64;
    let webhook = webhook_id.0 as i64;
    let count: Result<i32, sqlx::Error> = sqlx::query!(
        "
        SELECT COUNT(1) as count\n\
        FROM Connections\n\
        WHERE Connections.source = ? AND Connections.target = ? AND Connections.webhook = ?
        ",
        source,
        target,
        webhook
    )
    .fetch_one(db)
    .and_then(|row| async move { Ok(row.count) })
    .await;
    match count {
        Ok(count) => return Ok(count != 0),
        Err(why) => {
            println!("Error occured when trying to read connections from database: {why}");
            Err(why)
        }
    }
}

async fn maybe_add_connection(
    db: &SqlitePool,
    source_channel_id: &ChannelId,
    target_channel_id: &ChannelId,
    webhook_id: &WebhookId,
    user_id: &UserId,
) -> Result<bool, sqlx::Error> {
    match connection_exists(db, source_channel_id, target_channel_id, webhook_id).await {
        Ok(true) => return Ok(false),
        Err(why) => return Err(why),
        _ => (),
    }
    let source = source_channel_id.0 as i64;
    let target = target_channel_id.0 as i64;
    let webhook = webhook_id.0 as i64;
    let user = user_id.0 as i64;
    let result = sqlx::query!(
        "INSERT OR REPLACE INTO Connections (source, target, webhook, user) VALUES (?, ?, ?, ?)",
        source,
        target,
        webhook,
        user
    )
    .execute(db)
    .await;
    match result {
        Ok(_) => Ok(true),
        Err(why) => {
            println!("Failed to insert new connection into the database: {why}");
            Err(why)
        }
    }
}

async fn handle_connect_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
    ctx: &ClientContext,
) -> Result<CommandResponse> {
    let options = &command.data.options;
    let source = get_channel_opt("source", options)?;
    let target_server_name = get_string_opt("target_server", options)?;
    let target_channel_name = get_string_opt("target_channel", options)?;
    let (_target_server_id, target_channel_id) =
        name_to_ids(db, target_server_name, target_channel_name).await?;

    let webhook_id = maybe_add_webhook(db, &command.user.id, &target_channel_id, &ctx)
        .await
        .context(format!(
            "Internal error, failed to create webhook in <#{}>",
            target_channel_id.as_u64()
        ))?;

    let result = maybe_add_connection(
        db,
        &source.id,
        &target_channel_id,
        &webhook_id,
        &command.user.id,
    )
    .await?;

    match result {
        true => {
            let title = "Connection created".to_owned();
            let msg = format!(
                "Source: <#{}>\nTarget server: __**{}**__\nTarget channel: <#{}>",
                source.id,
                target_server_name,
                target_channel_id.as_u64()
            );
            Ok(CommandResponse { title, msg })
        }
        false => Err(anyhow!("Connection already exists")),
    }
}

async fn handle_disconnect_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
) -> Result<CommandResponse> {
    let options = &command.data.options;
    let source_channel = get_channel_opt("source", options)?;
    let combined = get_string_opt("target_channel", options)?;

    let re = Regex::new(r"\[(?P<server>.*)\] (?P<channel>.*)")?;
    let (target_server_name, target_channel_name) = match re.captures(combined) {
        Some(caps) => {
            let server_name = caps["server"].trim().to_owned();
            let channel_name = caps["channel"].trim().to_owned();
            (server_name, channel_name)
        }
        None => {
            bail!("Invalid target channel format\nIt has to be the following format: [<SERVER_NAME>] <CHANNEL_NAME>");
        }
    };

    let (_target_server_id, target_channel_id) =
        name_to_ids(db, &target_server_name, &target_channel_name).await?;

    let source = source_channel.id.0 as i64;
    let target = target_channel_id.0 as i64;
    let user = command.user.id.0 as i64;

    sqlx::query!(
        "DELETE FROM Connections WHERE source = ? AND target = ? AND user = ?",
        source,
        target,
        user
    )
    .execute(db)
    .await
    .map_err(|e| Error::new(e).context("Failed to delete connection in the database"))?;

    let title = "Disconnected".to_owned();
    let msg = format!(
        "Source: <#{}>\nServer: {target_server_name}\nTarget: <#{}>",
        source, target,
    );
    Ok(CommandResponse { title, msg })
}

async fn handle_disconnect_all_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
) -> Result<CommandResponse> {
    let options = &command.data.options;
    let source_channel = get_channel_opt("source", options)?;
    let source = source_channel.id.0 as i64;
    let user = command.user.id.0 as i64;

    sqlx::query!(
        "DELETE FROM Connections WHERE source = ? AND user = ?",
        source,
        user
    )
    .execute(db)
    .await
    .map_err(|e| Error::new(e).context("Failed to delete connections in the database"))?;

    let title = "Disconnected All".to_owned();
    let msg = format!("Source: <#{}>", source,);
    Ok(CommandResponse { title, msg })
}

async fn handle_list_connections_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
) -> Result<CommandResponse> {
    struct Connection {
        source: i64,
        target: i64,
        source_guild: String,
        target_guild: String,
    }

    impl From<Connection> for String {
        fn from(c: Connection) -> Self {
            format!(
                "> <#{}> => <#{}> **({})**",
                c.source, c.target, c.target_guild
            )
        }
    }

    let user = command.user.id.0 as i64;
    let connections: Vec<Connection> = sqlx::query!(
        "
        SELECT\n\
        user as \"user: i64\",\n\
        source as \"source: i64\",\n\
        target as \"target: i64\",\n\
        source_guild.name as source_guild,\n\
        target_guild.name as target_guild\n\
        FROM Connections\n\
        JOIN Channels source_channel\n\
        ON Connections.source = source_channel.id\n\
        JOIN Guilds source_guild\n\
        ON source_guild.id = source_channel.guild\n\
        JOIN Channels target_channel\n\
        ON Connections.target = target_channel.id\n\
        JOIN Guilds target_guild\n\
        ON target_guild.id = target_channel.guild\n\
        WHERE user = ?
        ",
        user
    )
    .fetch_all(db)
    .and_then(|records| async {
        Ok(records
            .into_iter()
            .map(|record| Connection {
                source: record.source,
                target: record.target,
                source_guild: record.source_guild,
                target_guild: record.target_guild,
            })
            .collect::<Vec<Connection>>())
    })
    .await
    .map_err(|e| {
        anyhow!(e).context("Failed to retrieve connections for server from the database")
    })?;

    let grouped = {
        let mut grouped: HashMap<String, Vec<Connection>> = HashMap::default();
        for c in connections {
            match grouped.get_mut(&c.source_guild) {
                Some(val) => val.push(c),
                None => {
                    let _ = grouped.insert(c.source_guild.clone(), vec![c]);
                }
            };
        }
        grouped
    };

    let msg = grouped
        .into_iter()
        .map(|(k, cs)| {
            let s = cs
                .into_iter()
                .map(String::from)
                .collect::<Vec<String>>()
                .join("\n");
            format!("__**{}**__\n{}", k, s)
        })
        .collect::<Vec<String>>()
        .join("\n\n");

    Ok(CommandResponse {
        title: "Connection List".to_owned(),
        msg,
    })
}

async fn handle_wipe_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
) -> Result<CommandResponse> {
    let options = &command.data.options;
    let server_name = get_string_opt("server", options)?;
    let user = command.user.id.0 as i64;

    sqlx::query!(
        "
        DELETE FROM Connections\n\
        WHERE Connections.id in (\n\
            SELECT Connections.id from Connections\n\
            JOIN Channels source_channel\n\
            ON Connections.source = source_channel.id\n\
            JOIN Guilds source_guild\n\
            ON source_guild.id = source_channel.guild\n\
            JOIN Channels target_channel\n\
            ON Connections.target = target_channel.id\n\
            JOIN Guilds target_guild\n\
            ON target_guild.id = target_channel.guild\n\
            WHERE (source_guild.name = ? OR target_guild.name = ?) AND user = ?\n\
        );
        ",
        server_name,
        server_name,
        user
    )
    .execute(db)
    .await
    .map_err(|e| Error::new(e).context("Failed to wipe connections for server in the database"))?;

    let title = "Wiped".to_owned();
    let msg = format!("Removed all connections: __**{server_name}**__");
    Ok(CommandResponse { title, msg })
}

async fn handle_application_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
    ctx: &ClientContext,
) {
    let result = match command.data.name.as_str() {
        "connect" => handle_connect_command(db, command, ctx).await,
        "disconnect" => handle_disconnect_command(db, command).await,
        "disconnect-all" => handle_disconnect_all_command(db, command).await,
        "list-connections" => handle_list_connections_command(db, command).await,
        "wipe" => handle_wipe_command(db, command).await,
        _ => Err(anyhow!(
            "Unknown command\n**{}**",
            command.data.name.as_str()
        )),
    };
    match result {
        Ok(rsp) => ok_command_response(&rsp.title, &rsp.msg, command, ctx).await,
        Err(e) => {
            println!("{:?}", e);
            error_command_response(&e.to_string(), command, ctx).await;
        }
    }
}

async fn initiate_database_connection() -> Option<SqlitePool> {
    let content = match tokio::fs::read_to_string(".env").await {
        Ok(db_name) => db_name,
        Err(err) => {
            println!(
                "\n{}\nCould not read the \".env\" file, make sure a file with this name\n\
                exists in the same directory as the bot (err: {})",
                style("Error:").red(),
                style(&err).cyan()
            );
            return None;
        }
    };
    let re = Regex::new(r"DATABASE_URL=sqlite:(?P<filename>.*)").unwrap();
    let db_name = match re.captures(&content) {
        Some(caps) => caps["filename"].trim().to_owned(),
        None => {
            println!(
                "\n{}\nCould not find the DB name in the \".env\" file, make sure it is one line\n\
                that says \"DATABASE_URL=sqlite:data.db\" or some other name for the DB file\n\
                (content: {})",
                style("Error:").red(),
                style(&content).cyan()
            );
            return None;
        }
    };
    return Some(
        sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(5)
            .connect_with(sqlx::sqlite::SqliteConnectOptions::new().filename(db_name))
            .await
            .unwrap(),
    );
}

#[tokio::main]
async fn main() {
    let data: Arc<Mutex<Data>> = Arc::new(Mutex::new(Data::default()));
    let (cache_rdy_tx, mut cache_rdy_rx) = tokio::sync::mpsc::channel::<bool>(1);

    let discord_token = match tokio::fs::read_to_string("token.txt").await {
        Err(err) => {
            println!(
                "\n{}\nCould not read the authentication token from \"token.txt\"\n\
                Make sure that the file exists and is located in the same\n\
                directory as the bot executable (err: {})",
                style("Error:").red(),
                style(err).cyan()
            );
            return;
        }
        Ok(discord_token) => {
            println!("Discord authentication token: {}", discord_token);
            discord_token
        }
    };

    let db = match initiate_database_connection().await {
        Some(db) => db,
        None => return,
    };

    // !HACK (this should be saved in the TOKEN file)
    let application_id: u64 = 936607788493307944;

    let mut client = Client::builder(&discord_token)
        .event_handler(Handler {
            data: data.clone(),
            db,
            cache_rdy_tx,
        })
        .application_id(application_id)
        .await
        .expect("Error creating Discord client");

    tokio::spawn(async move {
        if let Err(why) = client.start().await {
            println!("Discord client error: {why}");
            return;
        }
    });

    // Discord cache has been received and parsed.
    cache_rdy_rx.recv().await;

    let (exit_tx, mut exit_rx) = tokio::sync::mpsc::channel::<bool>(1);
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
                .unwrap_or("help".into());
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
                let rsp = handle_input(msg, data.clone()).await;
                main_tx.send(rsp).await.unwrap();
                exit_tx.send(rsp).await.unwrap();
            }
            Some(false) = exit_rx.recv() => {
                println!("Exiting...");
                break
            }
        }
    }
    cli_handle.join().unwrap();
}
