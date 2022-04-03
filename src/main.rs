use crate::ServerStatus::{Inactive, Running, Starting};
use async_process::Command as AsyncCommand;
use frankenstein::{Api, GetUpdatesParamsBuilder, Message, SendMessageParamsBuilder, TelegramApi};
use futures_lite::io::BufReader;
use futures_lite::{AsyncBufReadExt, StreamExt};
use json::JsonValue;
use regex::Regex;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::string::String;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::time::Duration;
use std::{fs, str};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::sleep;

type ChatbridgeMap = Arc<RwLock<HashMap<String, JoinHandle<()>>>>;

static CHAT_SERVER_MAP: &str = "chat_server_map";

#[tokio::main]
async fn main() {
    // Read configuration json
    let config_file = fs::read_to_string("bot-config.json").expect("Error reading config file");
    let config = json::parse(&config_file).expect("Error parsing json");
    let token = config["token"]
        .as_str()
        .expect("Error reading token from json");
    println!("Configs (incl. token) read successfully");
    let chatbridge_map: ChatbridgeMap = Arc::new(RwLock::new(HashMap::new()));

    // Construct api
    let api = Api::new(token);

    //let bot_name = api.get_me().await.unwrap().result.username.unwrap();

    let mut update_params_builder = GetUpdatesParamsBuilder::default();
    update_params_builder.allowed_updates(vec!["message".to_string()]);

    let mut update_params = update_params_builder.build().unwrap();

    println!("Start update loop.");
    loop {
        let result = api.get_updates(&update_params);

        match result {
            Ok(response) => {
                for update in response.result {
                    if let Some(message) = update.message {
                        if config[CHAT_SERVER_MAP].has_key(&message.chat.id.to_string()) {
                            println!(
                                "Message received from {:}, handling enabled.",
                                message.chat.id
                            );
                            let api_clone = api.clone();
                            let config_clone = config.clone();
                            let chatbridge_map_clone = chatbridge_map.clone();

                            tokio::spawn(async move {
                                process_message(
                                    message,
                                    api_clone,
                                    config_clone,
                                    chatbridge_map_clone,
                                )
                                .await;
                            });
                        } else {
                            println!(
                                "Message received from {:}, no handling enabled.",
                                message.chat.id
                            );
                        }

                        update_params = update_params_builder
                            .offset(update.update_id + 1)
                            .build()
                            .unwrap();
                    }
                }
            }
            Err(error) => {
                println!("Failed to get updates: {:?}", error);
            }
        }
    }
}

async fn process_message(
    message: Message,
    api: Api,
    config: JsonValue,
    chatbridge_map: ChatbridgeMap,
) {
    if let Some(text) = &message.text {
        if text.starts_with("/start_server") {
            start_server_handler(message, api, config).await;
        } else if text.starts_with("/stop_server") {
            stop_server_handler(message, api, config).await;
        } else if text.starts_with("/status_server") {
            status_server_handler(message, api, config).await;
        } else if text.starts_with("/enable_chatbridge") {
            enable_chatbridge_handler(message, api, config, chatbridge_map).await;
        } else if text.starts_with("/disable_chatbridge") {
            disable_chatbridge_handler(message, api, config, chatbridge_map).await;
        } else {
            pass_message_to_chatbridge(message, api, config, chatbridge_map).await;
        }
    }
}

async fn start_server_handler(message: Message, api: Api, config: JsonValue) {
    let server_name = config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
        .as_str()
        .expect("Error getting server name value");
    match get_service_active(&config, &message) {
        Inactive => {
            send_message_with_reply(&message, &api, "Ich starte den Server.").await;
            println!("Start server {:}.", server_name);
            let service_name = format!("minecraft-server@{:}.service", server_name);
            Command::new("sudo")
                .args(["systemctl", "start", &service_name])
                .spawn()
                .expect("Error executing command");

            let api_clone = api.clone();
            let finish = Arc::new(AtomicBool::new(false));
            let finish_clone = finish.clone();
            let message_clone = message.clone();
            let server_name_clone = String::from(server_name);

            let handle = tokio::spawn(async move {
                println!(
                    "Start thread to check online status of {:}.",
                    server_name_clone
                );
                let out = AsyncCommand::new("sudo")
                    .args(["journalctl", "-f", "-u", &service_name])
                    .stdout(Stdio::piped())
                    .spawn()
                    .unwrap();
                let mut reader = BufReader::new(out.stdout.unwrap()).lines();
                while let Some(line) = reader.next().await {
                    if line.unwrap().contains("]: Done") {
                        send_message_with_reply(
                            &message_clone,
                            &api_clone,
                            "Der Sterver ist nun gestartet.",
                        )
                        .await;
                        finish_clone.store(true, Relaxed);
                        break;
                    }
                }
                println!(
                    "Finished thread to check online status of {:}.",
                    server_name_clone
                );
            });

            for _ in 0..60 {
                sleep(Duration::from_secs(1)).await;
                if finish.load(Relaxed) {
                    break;
                }
            }
            if !finish.load(Relaxed) {
                handle.abort();
                send_message_with_reply(&message, &api, "Der Server wurde gestartet, allerdings kann nicht ermittelt werden, ob er nun auch läuft.").await;
            }
            println!(
                "Saw that {:} is online now, finishing handling of start_server.",
                server_name
            );
        }
        Starting => {
            println!("Server {:} already starting.", server_name);
            send_message_with_reply(&message, &api, "Der Server startet bereits.").await;
        }
        ServerStatus::Running { .. } => {
            println!("Server {:} already running.", server_name);
            send_message_with_reply(&message, &api, "Der Server läuft bereits.").await;
        }
    }
}

async fn stop_server_handler(message: Message, api: Api, config: JsonValue) {
    let server_name = config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
        .as_str()
        .expect("Error getting server name value");

    match get_service_active(&config, &message) {
        Inactive => {
            send_message_with_reply(&message, &api, "Der Server läuft derzeit nicht.").await;
            println!("Server {:} not running, cannot stop.", server_name);
        }
        Starting => {
            send_message_with_reply(&message, &api, "Der Server startet gerade. Bitte warte, bis der Server vollständig hochgefahren ist, bis du ihn stoppst.").await;
            println!("Server {:} currently starting, cannot stop.", server_name);
        }
        ServerStatus::Running { .. } => {
            send_message_with_reply(&message, &api, "Ich stoppe den Server.").await;
            println!("Stop server {:}.", server_name);
            Command::new("sudo")
                .args([
                    "systemctl",
                    "stop",
                    format!("minecraft-server@{:}.service", server_name).as_str(),
                ])
                .spawn()
                .expect("Error executing command");
        }
    }
}

async fn status_server_handler(message: Message, api: Api, config: JsonValue) {
    match get_service_active(&config, &message) {
        Inactive => {
            send_message_with_reply(&message, &api, "Der Server läuft gerade nicht.").await;
        }
        Starting => {
            send_message_with_reply(&message, &api, "Der Server startet gerade.").await;
        }
        ServerStatus::Running {
            current_players,
            max_players,
            players,
        } => {
            if current_players == "0" {
                send_message_with_reply(
                    &message,
                    &api,
                    "Der Server läuft gerade, aber niemand ist online.",
                )
                .await;
            } else {
                send_message_with_reply(
                    &message,
                    &api,
                    &format!(
                        "Der Server läuft gerade und es sind {:} von {:} Spieler:innen online: {:}",
                        current_players, max_players, players
                    ),
                )
                .await;
            }
        }
    }
}

async fn enable_chatbridge_handler(
    message: Message,
    api: Api,
    config: JsonValue,
    chatbridge_map: ChatbridgeMap,
) {
    if chatbridge_map
        .read()
        .await
        .contains_key(&message.chat.id.to_string())
    {
        println!(
            "Chat bridge for {} already activated.",
            &message.chat.id.to_string()
        );
        send_message_with_reply(&message, &api, "Die Chatbridge ist bereits aktiviert.").await;
    } else if let Running { .. } = get_service_active(&config, &message) {
        send_message_with_reply(&message, &api, "Die Chatbridge wird aktiviert.").await;
        println!(
            "Chat bridge will be activated for {}.",
            &message.chat.id.to_string()
        );
        let handle = tokio::spawn(async move {
            println!(
                "Start chatbridge thread for {}.",
                &message.chat.id.to_string()
            );
            let service_name = format!(
                "minecraft-server@{:}.service",
                config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
                    .as_str()
                    .expect("Error getting server name value")
            );
            let message_regex = Regex::new("INFO]: <([A-Za-z0-9]+)> (.*)").unwrap();
            let out = AsyncCommand::new("sudo")
                .args(["journalctl", "-f", "-n", "0", "-u", &service_name])
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let mut reader = BufReader::new(out.stdout.unwrap()).lines();
            while let Some(line) = reader.next().await {
                if let Some(captures) = message_regex.captures(&line.unwrap()) {
                    let send_message_params = SendMessageParamsBuilder::default()
                        .chat_id(message.chat.id)
                        .text(format!("<{}> {}", &captures[1], &captures[2]))
                        .build()
                        .unwrap();

                    if let Err(err) = api.send_message(&send_message_params) {
                        println!("Failed to send message: {:?}", err);
                    }
                }
            }
        });
        let mut chatbridge_lock = chatbridge_map.write().await;
        chatbridge_lock.insert(message.chat.id.to_string(), handle);
    }
}

async fn disable_chatbridge_handler(
    message: Message,
    api: Api,
    _: JsonValue,
    chatbridge_map: ChatbridgeMap,
) {
    if !chatbridge_map
        .read()
        .await
        .contains_key(&message.chat.id.to_string())
    {
        println!(
            "Chat bridge for {} not active.",
            &message.chat.id.to_string()
        );
        send_message_with_reply(&message, &api, "Die Chatbridge ist bereits deaktiviert.").await;
    } else {
        send_message_with_reply(&message, &api, "Die Chatbridge wird deaktiviert.").await;
        println!(
            "Chat bridge for {} gets deactivated.",
            &message.chat.id.to_string()
        );
        let mut chatbridge_lock = chatbridge_map.write().await;
        chatbridge_lock[&message.chat.id.to_string()].abort();
        chatbridge_lock.remove(&message.chat.id.to_string());
    }
}

async fn pass_message_to_chatbridge(
    message: Message,
    _: Api,
    config: JsonValue,
    chatbridge_map: ChatbridgeMap,
) {
    if chatbridge_map
        .read()
        .await
        .contains_key(&message.chat.id.to_string())
    {
        println!(
            "Received message for chatbridge for {}.",
            &message.chat.id.to_string()
        );
        let name = if let Some(username) = &message.from.as_ref().unwrap().username {
            username
        } else {
            &message.from.as_ref().unwrap().first_name
        };
        let text = message.text.as_ref().unwrap();
        Command::new("mcrcon")
            .args([
                "-H",
                "localhost",
                "-P",
                "25575",
                "-p",
                config["rcon_password"]
                    .as_str()
                    .expect("Error reading rcon password from json"),
                &format!(
                    "tellraw @a [\"\",{{\"text\":\"{}\",\"bold\":true}},\": {}\"]",
                    name, text
                ),
            ])
            .output()
            .expect("Error executing command");
    }
}

#[derive(PartialEq)]
enum ServerStatus {
    Inactive,
    Starting,
    Running {
        current_players: String,
        max_players: String,
        players: String,
    },
}

fn get_service_active(config: &JsonValue, message: &Message) -> ServerStatus {
    let server_name = config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
        .as_str()
        .expect("Error getting server name value");
    println!("Get status for server {:}.", server_name);
    let output = Command::new("sudo")
        .args([
            "systemctl",
            "is-active",
            format!("minecraft-server@{:}.service", server_name).as_str(),
        ])
        .output()
        .expect("Error executing command");
    if std::str::from_utf8(&output.stdout).expect("Error") == "active\n" {
        println!("Service for {:} is active.", server_name);
        let output = Command::new("mcrcon")
            .args([
                "-H",
                "localhost",
                "-P",
                "25575",
                "-p",
                config["rcon_password"]
                    .as_str()
                    .expect("Error reading rcon password from json"),
                "list",
            ])
            .output()
            .expect("Error executing command");
        if std::str::from_utf8(&output.stderr)
            .expect("Error")
            .contains("Connection failed")
        {
            println!("Server {:} is starting.", server_name);
            Starting
        } else {
            println!("Server {:} is online.", server_name);
            let text = std::str::from_utf8(&output.stdout).expect("Error");
            let re = Regex::new(r"[0-9]+").unwrap();
            let mut text_iter = re.captures_iter(text);
            let current_players = text_iter.next().unwrap();
            let max_players = text_iter.next().unwrap();
            let re: Vec<&str> = text.split(": ").collect();
            Running {
                current_players: String::from(&current_players[0]),
                max_players: String::from(&max_players[0]),
                players: String::from(&re[1][..re[1].len() - 5]),
            }
        }
    } else {
        println!("Service for server {:} is inactive.", server_name);
        Inactive
    }
}

async fn send_message_with_reply(message: &Message, api: &Api, reply: &str) {
    let send_message_params = SendMessageParamsBuilder::default()
        .chat_id(message.chat.id)
        .text(reply)
        .reply_to_message_id(message.message_id)
        .build()
        .unwrap();

    if let Err(err) = api.send_message(&send_message_params) {
        println!("Failed to send message: {:?}", err);
    }
}
