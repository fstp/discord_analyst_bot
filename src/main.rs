#![feature(hash_drain_filter)]
#![feature(io_error_other)]

use anyhow::{anyhow, bail, Context, Error, Result};
use console::style;
use futures::TryFutureExt;
use regex::Regex;
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
        webhook::Webhook,
    },
    prelude::*,
    utils::Color,
};
use sqlx::SqlitePool;
use std::{
    cmp,
    collections::HashMap,
    fmt::Display,
};
use sublime_fuzzy::best_match;

struct CommandResponse {
    title: String,
    msg: String,
}

struct AutocompleteResponse {
    options: Vec<String>,
}

async fn create_server_mapping(db: &SqlitePool, ctx: &ClientContext, id: &GuildId) -> Result<()> {
    let guild = id.0 as i64;
    let name = id
        .name(&ctx)
        .await
        .context(format!("Failed to get name from guild id: {guild}"))?;

    sqlx::query!(
        "INSERT INTO Guilds (id, name, is_banned) VALUES (?, ?, false)",
        guild,
        name,
    )
    .execute(db)
    .await
    .map_err(|_e| anyhow!("Guild already exists in the database: {name}"))?;

    let channels: Vec<(ChannelId, GuildChannel)> =
        id.channels(&ctx).await.unwrap().into_iter().collect();

    for (ch_id, ch) in channels {
        if ch.kind == ChannelType::Text {
            let webhook = ch.create_webhook(&ctx, "Analyst Bot").await.unwrap().id.0 as i64;
            let channel = ch_id.0 as i64;
            let name = format!("#{}", ch.name);
            sqlx::query!(
                "INSERT INTO Channels (id, name, guild, webhook) VALUES (?, ?, ?, ?)",
                channel,
                name,
                guild,
                webhook
            )
            .execute(db)
            .await
            .unwrap();
        }
    }

    Ok(())
}

async fn get_guild_names(db: &SqlitePool) -> Result<Vec<String>> {
    sqlx::query!("SELECT Guilds.name FROM Guilds")
        .fetch_all(db)
        .and_then(|result| async { Ok(result.into_iter().map(|record| record.name).collect()) })
        .await
        .map_err(|e| anyhow!(e).context("Failed to retrieve guild names from the database"))
}

// async fn get_guild_ids(db: &SqlitePool) -> Vec<GuildId> {
//     sqlx::query!("SELECT Guilds.id FROM Guilds")
//         .fetch_all(db)
//         .await
//         .unwrap()
//         .into_iter()
//         .map(|record| GuildId(record.id as u64))
//         .collect()
// }

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
    async fn ready(&self, _ctx: ClientContext, ready: Ready) {
        println!("{} is connected to Discord", ready.user.name);
    }

    async fn cache_ready(&self, ctx: ClientContext, guilds: Vec<GuildId>) {
        println!("Cache is ready");
        for id in &guilds {
            match create_server_mapping(&self.db, &ctx, &id).await {
                Ok(_) => (),
                Err(e) => println!("{:?}", e),
            }
        }
        println!("Server mapping created");
        for id in &guilds {
            let result = GuildId::set_application_commands(id, &ctx.http, |commands| {
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
                                    .channel_types(&[ChannelType::Text])
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
                                    .channel_types(&[ChannelType::Text])
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
                                    .channel_types(&[ChannelType::Text])
                            })
                    })
                    .create_application_command(|command| {
                        command
                            .name("list-connections")
                            .description("List all the active connections between all servers")
                    })
                    .create_application_command(|command| {
                        command
                            .name("wipe-connections")
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
                    .create_application_command(|command| {
                        command
                            .name("wipe-mentions")
                            .description(
                                "[WARNING] Will remove ALL mentions to/from channels in the selected server",
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
                    .create_application_command(|command| {
                        command
                            .name("mention-add")
                            .description("Add mentions to the target channel")
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
                            .create_option(|option| {
                                option
                                    .name("mentions")
                                    .description("One or more mentions separated by whitespace")
                                    .kind(ApplicationCommandOptionType::String)
                                    .required(true)
                            })
                            .create_option(|option| {
                                option
                                    .name("source")
                                    .description(
                                        "If set then only messages from this channel are mentioned",
                                    )
                                    .kind(ApplicationCommandOptionType::Channel)
                                    .required(false)
                                    .channel_types(&[ChannelType::Text])
                            })
                    })
                    .create_application_command(|command| {
                        command
                            .name("list-mentions")
                            .description("List all mentions for channels in the target server")
                            .create_option(|option| {
                                option
                                    .name("target_server")
                                    .description("Target server")
                                    .kind(ApplicationCommandOptionType::String)
                                    .required(true)
                                    //.set_autocomplete(true)
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
        println!("Slash commands added");
        self.cache_rdy_tx
            .send(true)
            .await
            .expect("Failed to send cache ready");
    }

    async fn message(&self, ctx: ClientContext, msg: Message) {
        match handle_message(&self.db, &ctx, &msg).await {
            Ok(_) => (),
            Err(e) => println!("{:?}", e),
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

async fn get_mentions(
    db: &SqlitePool,
    target: &ChannelId,
    source: &ChannelId,
    user: &UserId,
) -> Result<Vec<String>> {
    let target = target.0 as i64;
    let source = source.0 as i64;
    let user = user.0 as i64;

    let mut mentions: Vec<String> = sqlx::query!(
        "
        SELECT mention\n\
        FROM Mentions\n\
        WHERE (source IS NULL AND target = ? AND user = ?) OR (source = ? AND target = ? AND user = ?)
        ",
        target,
        user,
        source,
        target,
        user
    )
    .fetch_all(db)
    .and_then(|rows| async move { Ok(rows.into_iter().map(|row| row.mention).collect()) })
    .map_err(|e| Error::new(e).context("Failed to retrieve mentions (no source) from database"))
    .await?;

    mentions.sort_unstable();
    mentions.dedup();

    Ok(mentions)
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
        let webhook = id
            .to_webhook(&ctx)
            .await
            .context(format!("Failed to retrieve webhook from Discord: {id}"))?;
        let target = &webhook.channel_id;
        let source = &msg.channel_id;
        let mentions = get_mentions(db, target, source, &msg.author.id).await?;
        match execute_webhook(&webhook, ctx, msg, &mentions).await {
            Err(e) => println!("{:?}", e),
            _ => (),
        }
    }

    Ok(())
}

async fn execute_webhook(
    webhook: &Webhook,
    ctx: &ClientContext,
    msg: &Message,
    mentions: &Vec<String>,
) -> Result<()> {
    let avatar_url = match msg.author.avatar_url() {
        Some(url) => url,
        None => "".to_owned(),
    };
    // webhook
    //     .edit(
    //         &ctx,
    //         Some(&msg.author.name),
    //         Some(&image),
    //     )
    //     .await
    //     .context(format!("Failed to edit webhook:\n{:#?}", webhook))?;
    webhook
        .execute(&ctx, false, |w| {
            let embed = Embed::fake(|e| {
                e /*.author(|a| a.name(username).url(user_url).icon_url(icon_url))*/
                    .description(&msg.content)
                    .color(Color::GOLD)
            });
            w.username(&msg.author.name)
                .avatar_url(&avatar_url)
                .embeds(vec![embed])
                .content(mentions.join("\n"))
        })
        .await
        .context(format!("Failed to execute webhook:\n{:#?}", webhook))?;
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
        "wipe-connections" => handle_wipe_connections_autocomplete(db, autocomplete).await,
        "wipe-mentions" => handle_wipe_mentions_autocomplete(db, autocomplete).await,
        "mention-add" => handle_mention_add_autocomplete(db, autocomplete).await,
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

async fn handle_mention_add_autocomplete(
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

async fn handle_wipe_connections_autocomplete(
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

async fn handle_wipe_mentions_autocomplete(
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

// async fn get_webhook_id(
//     db: &SqlitePool,
//     user_id: &UserId,
//     target_channel_id: &ChannelId,
// ) -> Result<Option<WebhookId>> {
//     let user = user_id.0 as i64;
//     let target = target_channel_id.0 as i64;
//     sqlx::query!(
//         "
//         SELECT id as \"webhook_id: i64\"\n\
//         FROM Webhooks\n\
//         WHERE Webhooks.user = ? AND Webhooks.target = ?
//         ",
//         user,
//         target,
//     )
//     .fetch_optional(db)
//     .and_then(|row| async move { Ok(row.map(|row| WebhookId(row.webhook_id as u64))) })
//     .await
//     .map_err(|e| Error::new(e).context("Failed to retrieve webhook from database"))
// }

async fn connection_exists(
    db: &SqlitePool,
    source_channel_id: &ChannelId,
    target_channel_id: &ChannelId,
    user_id: &UserId,
) -> Result<bool> {
    let source = source_channel_id.0 as i64;
    let target = target_channel_id.0 as i64;
    let user = user_id.0 as i64;
    let count = sqlx::query!(
        "
        SELECT COUNT(1) as count\n\
        FROM Connections\n\
        WHERE source = ? AND target = ? AND user = ?
        ",
        source,
        target,
        user,
    )
    .fetch_one(db)
    .and_then(|row| async move { Ok(row.count) })
    .await
    .map_err(|e| Error::new(e).context("Failed to count existing connections in the database"))?;

    Ok(count != 0)
}

async fn maybe_add_connection(
    db: &SqlitePool,
    source_channel_id: &ChannelId,
    target_channel_id: &ChannelId,
    user_id: &UserId,
    webhook_id: &WebhookId,
) -> Result<bool> {
    match connection_exists(db, source_channel_id, target_channel_id, user_id).await {
        Ok(true) => return Ok(false),
        Err(why) => return Err(why),
        _ => (),
    }
    let source = source_channel_id.0 as i64;
    let target = target_channel_id.0 as i64;
    let user = user_id.0 as i64;
    let webhook = webhook_id.0 as i64;
    sqlx::query!(
        "INSERT INTO Connections (source, target, user, webhook) VALUES (?, ?, ?, ?)",
        source,
        target,
        user,
        webhook
    )
    .execute(db)
    .await
    .map_err(|e| Error::new(e).context("Failed to insert new connection into the database"))?;

    Ok(true)
}

async fn handle_connect_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
) -> Result<CommandResponse> {
    let options = &command.data.options;
    let source = get_channel_opt("source", options)?;
    let target_server_name = get_string_opt("target_server", options)?;
    let target_channel_name = get_string_opt("target_channel", options)?;
    let (_target_server_id, target_channel_id) =
        name_to_ids(db, target_server_name, target_channel_name).await?;

    let id = target_channel_id.0 as i64;
    let webhook_id = sqlx::query!("SELECT webhook FROM Channels WHERE id = ?", id)
        .fetch_one(db)
        .and_then(|record| async move { Ok(WebhookId(record.webhook as u64)) })
        .await?;

    let result = maybe_add_connection(
        db,
        &source.id,
        &target_channel_id,
        &command.user.id,
        &webhook_id,
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

async fn handle_list_mentions_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
) -> Result<CommandResponse> {
    struct Mentions {
        source: Option<i64>,
        target: i64,
        mentions: Vec<String>,
    }

    impl From<Mentions> for String {
        fn from(c: Mentions) -> Self {
            match c.source {
                Some(source) => {
                    format!(
                        "(**Boll's Server**) <#{}> => <#{}>\n> {}",
                        source,
                        c.target,
                        c.mentions.join("\n> ")
                    )
                }
                None => {
                    format!(
                        "(**ALL**) => <#{}>\n> {}",
                        c.target,
                        c.mentions.join("\n> ")
                    )
                }
            }
        }
    }

    let test_source = ChannelId(945744069596971021);
    let test_target = ChannelId(948272822441091144);
    let test_user = command.user.id;
    let mentions = get_mentions(db, &test_target, &test_source, &test_user).await?;

    let m = Mentions {
        source: None, //Some(test_source.0 as i64),
        target: test_target.0 as i64,
        mentions,
    };

    // async fn get_mentions(
    //     db: &SqlitePool,
    //     target: &ChannelId,
    //     source: &ChannelId,
    //     user: &UserId,
    // ) -> Result<Vec<String>> {

    Ok(CommandResponse {
        title: "Mention List for \"Boll's Server\"".to_owned(),
        msg: m.into(),
    })
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

async fn handle_wipe_connections_command(
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

    let title = "Wiped Connections".to_owned();
    let msg = format!("Removed all connections to/from: __**{server_name}**__");
    Ok(CommandResponse { title, msg })
}

async fn handle_wipe_mentions_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
) -> Result<CommandResponse> {
    let options = &command.data.options;
    let server_name = get_string_opt("server", options)?;
    let user = command.user.id.0 as i64;

    sqlx::query!(
        "
        DELETE FROM Mentions\n\
        WHERE Mentions.id in (\n\
            SELECT Mentions.id from Mentions\n\
            LEFT JOIN Channels source_channel\n\
            ON Mentions.source = source_channel.id\n\
            LEFT JOIN Guilds source_guild\n\
            ON source_guild.id = source_channel.guild\n\
            JOIN Channels target_channel\n\
            ON Mentions.target = target_channel.id\n\
            JOIN Guilds target_guild\n\
            ON target_guild.id = target_channel.guild\n\
            WHERE (source_guild.name = ? OR target_guild.name = ?) AND user = ?\n\
        );
        ",
        server_name,
        server_name,
        user,
    )
    .execute(db)
    .await
    .map_err(|e| Error::new(e).context("Failed to wipe mentions for server in the database"))?;

    let title = "Wiped Mentions".to_owned();
    let msg = format!("Removed all mentions to/from: __**{server_name}**__");
    Ok(CommandResponse { title, msg })
}

async fn mention_exists(
    db: &SqlitePool,
    source: &ChannelId,
    target: &ChannelId,
    mention: &str,
) -> Result<bool> {
    let source = source.0 as i64;
    let target = target.0 as i64;
    let count = sqlx::query!(
        "
        SELECT COUNT(1) as count\n\
        FROM Mentions\n\
        WHERE source = ? AND target = ? AND mention = ?
        ",
        source,
        target,
        mention
    )
    .fetch_one(db)
    .and_then(|row| async move { Ok(row.count) })
    .await
    .map_err(|e| Error::new(e).context("Failed to count existing mentions in the database"))?;

    Ok(count != 0)
}

async fn mention_exists_no_source(
    db: &SqlitePool,
    target: &ChannelId,
    mention: &str,
) -> Result<bool> {
    let target = target.0 as i64;
    let count = sqlx::query!(
        "
        SELECT COUNT(1) as count\n\
        FROM Mentions\n\
        WHERE source IS NULL AND target = ? AND mention = ?
        ",
        target,
        mention
    )
    .fetch_one(db)
    .and_then(|row| async move { Ok(row.count) })
    .await
    .map_err(|e| Error::new(e).context("Failed to count existing mentions in the database"))?;

    Ok(count != 0)
}

async fn handle_mention_add_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
) -> Result<CommandResponse> {
    let options = &command.data.options;
    let source = get_channel_opt("source", options);
    let target_server = get_string_opt("target_server", options)?;
    let target_channel = get_string_opt("target_channel", options)?;
    let mentions: Vec<&str> = get_string_opt("mentions", options)?.split(' ').collect();

    let (_target_server_id, target_channel_id) =
        name_to_ids(db, target_server, target_channel).await?;

    for m in &mentions {
        let user = command.user.id.0 as i64;
        let target = target_channel_id.0 as i64;

        if let Ok(ch) = source {
            let source = ch.id.0 as i64;
            let exists = mention_exists(db, &ch.id, &target_channel_id, m).await?;
            if !exists {
                let result = sqlx::query!(
                    "INSERT INTO Mentions (source, target, mention, user) VALUES (?, ?, ?, ?)",
                    source,
                    target,
                    m,
                    user
                )
                .execute(db)
                .await
                .map_err(|e| Error::new(e).context(format!("Failed to insert mention {m}")));
                match result {
                    Ok(_) => (),
                    Err(e) => println!("{e}"),
                };
            }
        } else {
            // No source channel provided.
            let exists = mention_exists_no_source(db, &target_channel_id, m).await?;
            if !exists {
                let result = sqlx::query!(
                    "INSERT INTO Mentions (source, target, mention, user) VALUES (NULL, ?, ?, ?)",
                    target,
                    m,
                    user
                )
                .execute(db)
                .await
                .map_err(|e| Error::new(e).context(format!("Failed to insert mention {m}")));
                match result {
                    Ok(_) => (),
                    Err(e) => println!("{e}"),
                };
            }
        }
    }

    let from_source = if let Ok(ch) = source {
        format!("\nSource channel: <#{}>", ch.id)
    } else {
        "".to_owned()
    };

    Ok(CommandResponse {
        title: "Added Mentions".to_owned(),
        msg: format!(
            "Mentions:\n{}\n\nTarget server: __**{}**__\nTarget channel <#{}>{}",
            mentions.join("\n"),
            target_server,
            target_channel_id,
            from_source
        ),
    })
}

async fn handle_application_command(
    db: &SqlitePool,
    command: &ApplicationCommandInteraction,
    ctx: &ClientContext,
) {
    let result = match command.data.name.as_str() {
        "connect" => handle_connect_command(db, command).await,
        "disconnect" => handle_disconnect_command(db, command).await,
        "disconnect-all" => handle_disconnect_all_command(db, command).await,
        "list-connections" => handle_list_connections_command(db, command).await,
        "wipe-connections" => handle_wipe_connections_command(db, command).await,
        "wipe-mentions" => handle_wipe_mentions_command(db, command).await,
        "mention-add" => handle_mention_add_command(db, command).await,
        "list-mentions" => handle_list_mentions_command(db, command).await,
        _ => Err(anyhow!(
            "Unknown command: **{}**",
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

    let mut client = Client::builder(&discord_token.trim())
        .event_handler(Handler {
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

    let (_exit_tx, mut exit_rx) = tokio::sync::mpsc::channel::<bool>(1);

    // Main event loop.
    loop {
        tokio::select! {
            Some(false) = exit_rx.recv() => {
                println!("Exiting...");
                break
            }
        }
    }
}
