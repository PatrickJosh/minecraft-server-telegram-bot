use frankenstein::{
    AsyncApi, AsyncTelegramApi, GetUpdatesParamsBuilder, Message, SendMessageParamsBuilder,
};
use json::JsonValue;
use regex::Regex;
use std::fs;
use std::process::Command;

static CHAT_SERVER_MAP: &str = "chat_server_map";

#[tokio::main]
async fn main() {
    // Read configuration json
    let config_file = fs::read_to_string("bot-config.json").expect("Error reading config file");
    let config = json::parse(&config_file).expect("Error parsing json");
    let token = config["token"]
        .as_str()
        .expect("Error reading token from json");

    // Construct api
    let api = AsyncApi::new(token);

    //let bot_name = api.get_me().await.unwrap().result.username.unwrap();

    let mut update_params_builder = GetUpdatesParamsBuilder::default();
    update_params_builder.allowed_updates(vec!["message".to_string()]);

    let mut update_params = update_params_builder.build().unwrap();

    loop {
        let result = api.get_updates(&update_params).await;

        match result {
            Ok(response) => {
                for update in response.result {
                    if let Some(message) = update.message {
                        if config[CHAT_SERVER_MAP].has_key(&message.chat.id.to_string()) {
                            let api_clone = api.clone();
                            let config_clone = config.clone();

                            tokio::spawn(async move {
                                process_message(message, api_clone, config_clone).await;
                            });
                        } else {
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

async fn process_message(message: Message, api: AsyncApi, config: JsonValue) {
    if let Some(text) = &message.text {
        if text.starts_with("/start_server") {
            tokio::spawn(async move {
                start_server_handler(message, api, config).await;
            });
        } else if text.starts_with("/stop_server") {
            tokio::spawn(async move {
                stop_server_handler(message, api, config).await;
            });
        } else if text.starts_with("/status_server") {
            tokio::spawn(async move {
                status_server_handler(message, api, config).await;
            });
        }
    }
}

async fn start_server_handler(message: Message, api: AsyncApi, config: JsonValue) {
    let server_name = config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
        .as_str()
        .expect("Error getting server name value");
    send_message_with_reply(message, api, "Ich starte den Server.").await;
    Command::new("sudo")
        .args([
            "systemctl",
            "start",
            format!("minecraft-server@{:}.service", server_name).as_str(),
        ])
        .spawn()
        .expect("Error executing command");
}

async fn stop_server_handler(message: Message, api: AsyncApi, config: JsonValue) {
    let server_name = config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
        .as_str()
        .expect("Error getting server name value");
    send_message_with_reply(message, api, "Ich stoppe den Server.").await;
    Command::new("sudo")
        .args([
            "systemctl",
            "stop",
            format!("minecraft-server@{:}.service", server_name).as_str(),
        ])
        .spawn()
        .expect("Error executing command");
}

async fn status_server_handler(message: Message, api: AsyncApi, config: JsonValue) {
    let server_name = config[CHAT_SERVER_MAP][&message.chat.id.to_string()]
        .as_str()
        .expect("Error getting server name value");
    let output = Command::new("sudo")
        .args([
            "systemctl",
            "is-active",
            format!("minecraft-server@{:}.service", server_name).as_str(),
        ])
        .output()
        .expect("Error executing command");
    if std::str::from_utf8(&output.stdout).expect("Error") == "active\n" {
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
            send_message_with_reply(message, api, "Der Server startet gerade.").await;
        } else {
            let text = std::str::from_utf8(&output.stdout).expect("Error");
            let re = Regex::new(r"[0-9]+").unwrap();
            let mut text_iter = re.captures_iter(text);
            let current_players = text_iter.next().unwrap();
            let max_players = text_iter.next().unwrap();
            if &current_players[0] == "0" {
                send_message_with_reply(
                    message,
                    api,
                    "Der Server läuft gerade, aber niemand ist online.",
                )
                .await;
            } else {
                let re: Vec<&str> = text.split(": ").collect();
                send_message_with_reply(
                    message,
                    api,
                    &format!(
                        "Der Server läuft gerade und es sind {:} von {:} Spieler:innen online: {:}",
                        &current_players[0],
                        &max_players[0],
                        &re[1][..re[1].len() - 5]
                    ),
                )
                .await;
            }
        }
    } else {
        send_message_with_reply(message, api, "Der Server läuft gerade nicht.").await;
    }
}

async fn send_message_with_reply(message: Message, api: AsyncApi, reply: &str) {
    let send_message_params = SendMessageParamsBuilder::default()
        .chat_id(message.chat.id)
        .text(reply)
        .reply_to_message_id(message.message_id)
        .build()
        .unwrap();

    if let Err(err) = api.send_message(&send_message_params).await {
        println!("Failed to send message: {:?}", err);
    }
}
