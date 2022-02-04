#![feature(hash_drain_filter)]

use console::style;
use dialoguer::Input;
use itertools::Itertools;
use rillrate::prime::{
    table::{Col, Row},
    *,
};
use serde::{Deserialize, Serialize};
use serenity::{
    async_trait,
    client::bridge::gateway::{ShardId, ShardManager},
    framework::standard::{
        macros::{command, group, hook},
        CommandResult, StandardFramework,
    },
    model::channel::{ChannelType, Message},
    model::gateway::Ready,
    model::id::{ChannelId, GuildId, WebhookId},
    prelude::*,
};
use std::{
    collections::{HashMap, HashSet},
    sync::{atomic::*, Arc},
    time::{Instant, Duration},
    thread,
};
use tokio::fs;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

// Name used to group dashboards.
// You could have multiple packages for different applications, such as a package for the bot
// dashboards, and another package for a web server running alongside the bot.
const PACKAGE: &str = "Bot Dashboards";
// Dashboards are a part inside of package, they can be used to group different types of
// dashboards that you may want to use, like a dashboard for system status, another dashboard
// for cache status, and another one to configure features or trigger actions on the bot.
const DASHBOARD_STATS: &str = "Statistics";
const DASHBOARD_CONFIG: &str = "Config Dashboard";
// This are collapsable menus inside the dashboard, you can use them to group specific sets
// of data inside the same dashboard.
// If you are using constants for this, make sure they don't end in _GROUP or _COMMAND, because
// serenity's command framework uses these internally.
const GROUP_LATENCY: &str = "1 - Discord Latency";
const GROUP_COMMAND_COUNT: &str = "2 - Command Trigger Count";
const GROUP_CONF: &str = "1 - Switch Command Configuration";
// All of the 3 configurable namescapes are sorted alphabetically.

#[derive(Debug, Clone)]
struct CommandUsageValue {
    index: usize,
    use_count: usize,
}

struct Components {
    data_switch: AtomicBool,
    double_link_value: AtomicU8,
    ws_ping_history: Pulse,
    get_ping_history: Pulse,
    #[cfg(feature = "post-ping")]
    post_ping_history: Pulse,
    command_usage_table: Table,
    command_usage_values: Mutex<HashMap<&'static str, CommandUsageValue>>,
}

struct RillRateComponents;

impl TypeMapKey for RillRateComponents {
    // RillRate element types have internal mutability, so we don't need RwLock nor Mutex!
    // We do still want to Arc the type so it can be cloned out of `ctx.data`.
    // If you wanna bind data between RillRate and the bot that doesn't have Atomics, use fields
    // that use RwLock or Mutex, rather than making the enirety of Components one of them, like
    // it's being done with `command_usage_values` this will make it considerably less likely
    // to deadlock.
    type Value = Arc<Components>;
}

struct ShardManagerContainer;

impl TypeMapKey for ShardManagerContainer {
    type Value = Arc<Mutex<ShardManager>>;
}

const WEBHOOK_NAME: &str = "Analyst Bot (QkTdmq49PE)";

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

async fn regenerate_webhooks(ctx: &Context, server: &Server) {
    for (_name, id) in &server.channels {
        let hooks = id
            .to_channel(&ctx)
            .await
            .unwrap()
            .guild()
            .unwrap()
            .webhooks(&ctx)
            .await
            .unwrap();
        let mut found = false;
        for hook in hooks {
            if hook.name == Some(WEBHOOK_NAME.to_owned()) {
                found = true;
                break;
            }
        }
        if found == false {
            // Webhook for this channel doesn't exist so we create it.
            id.to_channel(&ctx)
                .await
                .unwrap()
                .guild()
                .unwrap()
                .create_webhook(&ctx, WEBHOOK_NAME)
                .await
                .unwrap();
        }
    }
}

async fn create_server_mapping(data: &Arc<Mutex<Data>>, ctx: &Context, guilds: &Vec<GuildId>) {
    let mut data = data.lock().await;
    for id in guilds {
        let name = id.name(&ctx.cache).await.unwrap();
        let tag = data.next_server_tag.to_string();
        let channels: HashMap<String, ChannelId> = id
            .to_guild_cached(ctx.cache.clone())
            .await
            .unwrap()
            .channels
            .into_iter()
            .filter(|(_channel_id, channel)| channel.kind == ChannelType::Text)
            .map(|(channel_id, channel)| (channel.name, channel_id))
            .collect();
        let server = Server {
            name: name,
            id: *id,
            channels: channels,
        };
        regenerate_webhooks(&ctx, &server).await;
        data.server_mapping.insert(tag, server);
        data.next_server_tag += 1;
    }
    println!("Finished server mapping\n{:#?}", data);
}

async fn initiate_rillrate(ctx: Context) {
    let switch = Switch::new(
        [PACKAGE, DASHBOARD_CONFIG, GROUP_CONF, "Toggle Switch"],
        SwitchOpts::default().label("Switch Me and run the `~switch` command!"),
    );
    let switch_instance = switch.clone();

    let ctx_clone = ctx.clone();

    tokio::spawn(async move {
        // There's currently no way to read the current data stored on RillRate types,
        // so we use our own external method of storage, in this case since a switch is
        // essentially just a boolean, we use an AtomicBool, stored on the same
        // Components structure.
        let elements = {
            let data_read = ctx_clone.data.read().await;
            data_read.get::<RillRateComponents>().unwrap().clone()
        };

        switch.sync_callback(move |envelope| {
            if let Some(action) = envelope.action {
                println!("Switch action: {:?}", action);

                // Here we toggle our internal state for the switch.
                elements.data_switch.swap(action, Ordering::Relaxed);

                // If you click the switch, it won't turn on by itself, it will just send an
                // event about it's new status.
                // We need to manually set the switch to that status.
                // If we do it at the end, we can make sure the switch switches it's status
                // only if the action was successful.
                switch_instance.apply(action);
            }

            Ok(())
        });
    });

    let default_values = {
        let mut values = vec![];
        for i in u8::MIN..=u8::MAX {
            if i % 32 == 0 {
                values.push(i.to_string())
            }
        }
        values
    };

    // You are also able to have different actions in different elements interact with
    // the same data.
    // In this example, we have a Selector with preset data, and a Slider for more fine
    // grain control of the value.
    let selector = Selector::new(
        [PACKAGE, DASHBOARD_CONFIG, GROUP_CONF, "Value Selector"],
        SelectorOpts::default()
            .label("Select from a preset of values!")
            .options(default_values),
    );
    let selector_instance = selector.clone();

    let slider = Slider::new(
        [PACKAGE, DASHBOARD_CONFIG, GROUP_CONF, "Value Slider"],
        SliderOpts::default()
            .label("Or slide me for more fine grain control!")
            .min(u8::MIN as f64)
            .max(u8::MAX as f64)
            .step(2),
    );
    let slider_instance = slider.clone();

    let ctx_clone = ctx.clone();

    tokio::spawn(async move {
        let elements = {
            let data_read = ctx_clone.data.read().await;
            data_read.get::<RillRateComponents>().unwrap().clone()
        };

        selector.sync_callback(move |envelope| {
            let mut value: Option<u8> = None;

            if let Some(action) = envelope.action {
                println!("Values action (selector): {:?}", action);
                value = action.map(|val| val.parse().unwrap());
            }

            if let Some(val) = value {
                elements.double_link_value.swap(val, Ordering::Relaxed);

                // This is the selector callback, yet we are switching the data from the
                // slider, this is to make sure both fields share the same look in the
                // dashboard.
                slider_instance.apply(val as f64);
            }

            // the sync_callback() closure wants a Result value returned.
            Ok(())
        });
    });

    let ctx_clone = ctx.clone();

    tokio::spawn(async move {
        let elements = {
            let data_read = ctx_clone.data.read().await;
            data_read.get::<RillRateComponents>().unwrap().clone()
        };

        // Because sync_callback() waits for an action to happen to it's element,
        // we cannot have both in the same thread, rather we need to listen to them in
        // parallel, but still have both modify the same value in the end.
        slider.sync_callback(move |envelope| {
            let mut value: Option<u8> = None;

            if let Some(action) = envelope.action {
                println!("Values action (slider): {:?}", action);
                value = Some(action as u8);
            }

            if let Some(val) = value {
                elements.double_link_value.swap(val, Ordering::Relaxed);

                selector_instance.apply(Some(val.to_string()));
            }

            Ok(())
        });
    });

    let ctx_clone = ctx.clone();

    tokio::spawn(async move {
        let elements = {
            let data_read = ctx_clone.data.read().await;
            data_read.get::<RillRateComponents>().unwrap().clone()
        };

        loop {
            // Get the REST GET latency by counting how long it takes to do a GET request.
            let get_latency = {
                let now = Instant::now();
                // `let _` to supress any errors. If they are a timeout, that will  be
                // reflected in the plotted graph.
                let _ = reqwest::get("https://discordapp.com/api/v6/gateway").await;
                now.elapsed().as_millis() as f64
            };

            // POST Request is feature gated because discord doesn't like bots doing repeated
            // tasks in short time periods, as they are considered API abuse; this is specially
            // true on bigger bots. If you still wanna see this function though, compile the
            // code adding `--features post-ping` to the command.
            //
            // Get the REST POST latency by posting a message to #testing.
            //
            // If you don't want to spam, use the DM channel of some random bot, or use some
            // other kind of POST request such as reacting to a message, or creating an invite.
            // Be aware that if the http request fails, the latency returned may be incorrect.
            #[cfg(feature = "post-ping")]
            let post_latency = {
                let now = Instant::now();
                let _ = ChannelId(381926291785383946)
                    .say(&ctx_clone, "Latency Test")
                    .await;
                now.elapsed().as_millis() as f64
            };

            // Get the Gateway Heartbeat latency.
            // See example 5 for more information about the ShardManager latency.
            let ws_latency = {
                let data_read = ctx.data.read().await;
                let shard_manager = data_read.get::<ShardManagerContainer>().unwrap();

                let manager = shard_manager.lock().await;
                let runners = manager.runners.lock().await;

                let runner = runners.get(&ShardId(ctx.shard_id)).unwrap();

                if let Some(duration) = runner.latency {
                    duration.as_millis() as f64
                } else {
                    f64::NAN // effectively 0.0ms, it won't display on the graph.
                }
            };

            elements.ws_ping_history.push(ws_latency);
            elements.get_ping_history.push(get_latency);
            #[cfg(feature = "post-ping")]
            elements.post_ping_history.push(post_latency);

            // Update every heartbeat, when the ws latency also updates.
            tokio::time::sleep(Duration::from_millis(42500)).await;
        }
    });
}

#[group]
#[commands(ping, switch)]
struct General;

struct Handler {
    data: Arc<Mutex<Data>>,
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
    //
    // In this case, just print what the current user's username is.
    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} is connected to Discord", ready.user.name);
    }

    async fn cache_ready(&self, ctx: Context, guilds: Vec<GuildId>) {
        create_server_mapping(&self.data, &ctx, &guilds).await;
        initiate_rillrate(ctx).await;
        self.cache_rdy_tx
            .send(true)
            .await
            .expect("Failed to send cache ready");
    }

    async fn message(&self, ctx: Context, msg: Message) {
        if msg.author.bot == false {
            let data = self.data.lock().await;
            match data.source_channels.get(&msg.channel_id) {
                Some(source_channel) => {
                    match data.channel_mapping.get(&source_channel.channel_tag) {
                        Some(target_ids) => {
                            for id in target_ids {
                                let webhooks = id
                                    .to_channel(&ctx)
                                    .await
                                    .unwrap()
                                    .guild()
                                    .unwrap()
                                    .webhooks(&ctx)
                                    .await
                                    .unwrap();
                                for hook in &webhooks {
                                    match &hook.name {
                                        Some(name) => {
                                            if name == WEBHOOK_NAME {
                                                // Found our webhook, execute it!
                                                hook.execute(&ctx, false, |w| {
                                                    w.content(&msg.content);
                                                    w
                                                })
                                                .await
                                                .unwrap();
                                            }
                                        }
                                        None => {
                                            // Couldn't get the name of the webhook,
                                            // must not be ours then...
                                        }
                                    }
                                }
                            }
                        }
                        None => {
                            // No mapping (no targets) exist for this
                            // source channel so we ignore the message.
                        }
                    }
                }
                None => {
                    // Originating channel is not a source
                    // so we ignore the message.
                }
            }
        }
    }
}


#[hook]
async fn before_hook(ctx: &Context, _: &Message, cmd_name: &str) -> bool {
    let elements = {
        let data_read = ctx.data.read().await;
        data_read.get::<RillRateComponents>().unwrap().clone()
    };

    let command_count_value = {
        let mut count_write = elements.command_usage_values.lock().await;
        let mut command_count_value = count_write.get_mut(cmd_name).unwrap();
        command_count_value.use_count += 1;
        command_count_value.clone()
    };

    elements.command_usage_table.set_cell(
        Row(command_count_value.index as u64),
        Col(1),
        command_count_value.use_count,
    );

    println!("Running command {}", cmd_name);
    true
}

#[tokio::main]
async fn main() {
    let data: Arc<Mutex<Data>> = Arc::new(Mutex::new(Data::default()));
    let (cache_rdy_tx, mut cache_rdy_rx) = tokio::sync::mpsc::channel::<bool>(1);

    let discord_token = match tokio::fs::read_to_string("token.txt").await {
        Err(_) => {
            println!(
                "Could not read the authentication token from \"token.txt\"\n \
                Make sure that the file exists and is located in the same\n \
                directory as the bot executable"
            );
            return;
        }
        Ok(discord_token) => {
            println!("Discord authentication token: {}", discord_token);
            discord_token
        }
    };

    let mut client = Client::builder(&discord_token)
        .event_handler(Handler {
            data: data.clone(),
            cache_rdy_tx: cache_rdy_tx,
        })
        .await
        .expect("Error creating Discord client");

    tokio::spawn(async move {
        if let Err(why) = client.start().await {
            println!("Client error: {:?}", why);
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
