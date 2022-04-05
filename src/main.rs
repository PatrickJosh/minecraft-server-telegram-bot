use crate::ServerStatus::{Inactive, Running, Starting};
use async_process::Command as AsyncCommand;
use frankenstein::MessageEntityBuilder;
use frankenstein::MessageEntityType::Bold;
use frankenstein::{Api, GetUpdatesParamsBuilder, Message, SendMessageParamsBuilder, TelegramApi};
use futures_lite::io::BufReader;
use futures_lite::{AsyncBufReadExt, StreamExt};
use json::JsonValue;
use regex::Regex;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::string::String;
use std::sync::mpsc;
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

    let bot_data = BotData::new(api, config, chatbridge_map);

    println!("Start update loop.");
    loop {
        let result = bot_data.api.get_updates(&update_params);

        match result {
            Ok(response) => {
                for update in response.result {
                    if let Some(message) = update.message {
                        if bot_data.config[CHAT_SERVER_MAP].has_key(&message.chat.id.to_string()) {
                            println!(
                                "Message received from {:}, handling enabled.",
                                message.chat.id
                            );
                            let mut bot_data_clone = bot_data.clone();

                            tokio::spawn(async move {
                                bot_data_clone.process_message(message).await;
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

#[derive(Debug, Clone)]
struct BotData {
    api: Api,
    config: JsonValue,
    chatbridge_map: ChatbridgeMap,
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

impl BotData {
    fn new(api: Api, config: JsonValue, chatbridge_map: ChatbridgeMap) -> BotData {
        BotData {
            api,
            config,
            chatbridge_map,
        }
    }

    async fn process_message(&mut self, message: Message) {
        if let Some(text) = &message.text {
            if text.starts_with("/start_server") {
                self.start_server_handler(message).await;
            } else if text.starts_with("/stop_server") {
                self.stop_server_handler(message).await;
            } else if text.starts_with("/status_server") {
                self.status_server_handler(message).await;
            } else if text.starts_with("/enable_chatbridge") {
                self.enable_chatbridge_handler(message).await;
            } else if text.starts_with("/disable_chatbridge") {
                self.disable_chatbridge_handler(message, true).await;
            } else {
                self.pass_message_to_chatbridge(message).await;
            }
        }
    }

    async fn start_server_handler(&self, message: Message) {
        let server_name = self.config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
            .as_str()
            .expect("Error getting server name value");
        match self.get_service_active(&message) {
            Inactive => {
                self.send_message_with_reply(&message, "Ich starte den Server.")
                    .await;
                println!("Start server {:}.", server_name);
                let service_name = format!("minecraft-server@{:}.service", server_name);
                Command::new("sudo")
                    .args(["systemctl", "start", &service_name])
                    .spawn()
                    .expect("Error executing command");

                let message_clone = message.clone();
                let server_name_clone = String::from(server_name);
                let bot_data = self.clone();

                let (tx, rx) = mpsc::channel();

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
                            bot_data
                                .send_message_with_reply(
                                    &message_clone,
                                    "Der Server ist nun gestartet.",
                                )
                                .await;
                            match tx.send("finished") {
                                Ok(_) => {}
                                Err(_) => {
                                    println!("Main thread of starting server finished before this. Should not be reached.")
                                }
                            }
                            break;
                        }
                    }
                    println!(
                        "Finished thread to check online status of {:}.",
                        server_name_clone
                    );
                });

                let mut server_done = false;
                for _ in 0..60 {
                    sleep(Duration::from_secs(1)).await;
                    if let Ok("finished") = rx.try_recv() {
                        server_done = true;
                        break;
                    }
                }
                if !server_done {
                    handle.abort();
                    self.send_message_with_reply(&message, "Der Server wurde gestartet, allerdings kann nicht ermittelt werden, ob er nun auch läuft.").await;
                }
                println!(
                    "Finishing handling of start_server. Server {} was started properly: {}",
                    server_name, server_done
                );
            }
            Starting => {
                println!("Server {:} already starting.", server_name);
                self.send_message_with_reply(&message, "Der Server startet bereits.")
                    .await;
            }
            ServerStatus::Running { .. } => {
                println!("Server {:} already running.", server_name);
                self.send_message_with_reply(&message, "Der Server läuft bereits.")
                    .await;
            }
        }
    }

    async fn stop_server_handler(&mut self, message: Message) {
        let server_name = self.config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
            .as_str()
            .expect("Error getting server name value");

        match self.get_service_active(&message) {
            Inactive => {
                self.send_message_with_reply(&message, "Der Server läuft derzeit nicht.")
                    .await;
                println!("Server {:} not running, cannot stop.", server_name);
            }
            Starting => {
                self.send_message_with_reply(&message, "Der Server startet gerade. Bitte warte, bis der Server vollständig hochgefahren ist, bis du ihn stoppst.").await;
                println!("Server {:} currently starting, cannot stop.", server_name);
            }
            ServerStatus::Running { .. } => {
                self.send_message_with_reply(&message, "Ich stoppe den Server.")
                    .await;
                println!("Stop server {:}.", server_name);
                self.disable_chatbridge_handler(message.clone(), false)
                    .await;
                let server_name = self.config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
                    .as_str()
                    .expect("Error getting server name value");
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

    async fn status_server_handler(&self, message: Message) {
        match self.get_service_active(&message) {
            Inactive => {
                self.send_message_with_reply(&message, "Der Server läuft gerade nicht.")
                    .await;
            }
            Starting => {
                self.send_message_with_reply(&message, "Der Server startet gerade.")
                    .await;
            }
            ServerStatus::Running {
                current_players,
                max_players,
                players,
            } => {
                if current_players == "0" {
                    self.send_message_with_reply(
                        &message,
                        "Der Server läuft gerade, aber niemand ist online.",
                    )
                    .await;
                } else {
                    self.send_message_with_reply(
                        &message,
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

    async fn enable_chatbridge_handler(&mut self, message: Message) {
        if self
            .chatbridge_map
            .read()
            .await
            .contains_key(&message.chat.id.to_string())
        {
            println!(
                "Chat bridge for {} already activated.",
                &message.chat.id.to_string()
            );
            self.send_message_with_reply(&message, "Die Chatbridge ist bereits aktiviert.")
                .await;
        } else if let Running { .. } = self.get_service_active(&message) {
            self.send_message_with_reply(&message, "Die Chatbridge wird aktiviert.")
                .await;
            println!(
                "Chat bridge will be activated for {}.",
                &message.chat.id.to_string()
            );
            let message_clone = message.clone();
            let bot_data = self.clone();
            let handle = tokio::spawn(async move {
                let message = message_clone;
                println!(
                    "Start chatbridge thread for {}.",
                    &message.chat.id.to_string()
                );
                let service_name = format!(
                    "minecraft-server@{:}.service",
                    bot_data.config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
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
                            .text(format!("{}: {}", &captures[1], &captures[2]))
                            .entities(vec![MessageEntityBuilder::default()
                                .type_field(Bold)
                                .offset(0_u16)
                                .length(captures[1].len() as u16)
                                .build()
                                .unwrap()])
                            .build()
                            .unwrap();

                        if let Err(err) = bot_data.api.send_message(&send_message_params) {
                            println!("Failed to send message: {:?}", err);
                        }
                    }
                }
            });
            let mut chatbridge_lock = self.chatbridge_map.write().await;
            chatbridge_lock.insert(message.chat.id.to_string(), handle);
        } else {
            self.send_message_with_reply(&message, "Der Server läuft gerade nicht oder startet gerade, daher kann die Chatbridge nicht gestartet werden.")
                .await;
        }
    }

    async fn disable_chatbridge_handler(&mut self, message: Message, send_message: bool) {
        if !self
            .chatbridge_map
            .read()
            .await
            .contains_key(&message.chat.id.to_string())
        {
            println!(
                "Chat bridge for {} not active.",
                &message.chat.id.to_string()
            );
            if send_message {
                self.send_message_with_reply(&message, "Die Chatbridge ist bereits deaktiviert.")
                    .await;
            }
        } else {
            if send_message {
                self.send_message_with_reply(&message, "Die Chatbridge wird deaktiviert.")
                    .await;
            }
            println!(
                "Chat bridge for {} gets deactivated.",
                &message.chat.id.to_string()
            );
            let mut chatbridge_lock = self.chatbridge_map.write().await;
            chatbridge_lock[&message.chat.id.to_string()].abort();
            chatbridge_lock.remove(&message.chat.id.to_string());
        }
    }

    async fn pass_message_to_chatbridge(&mut self, message: Message) {
        if self
            .chatbridge_map
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
                    self.config["rcon_password"]
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

    fn get_service_active(&self, message: &Message) -> ServerStatus {
        let server_name = self.config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
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
                    self.config["rcon_password"]
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

    async fn send_message_with_reply(&self, message: &Message, reply: &str) {
        let send_message_params = SendMessageParamsBuilder::default()
            .chat_id(message.chat.id)
            .text(reply)
            .reply_to_message_id(message.message_id)
            .build()
            .unwrap();

        if let Err(err) = self.api.send_message(&send_message_params) {
            println!("Failed to send message: {:?}", err);
        }
    }
}
