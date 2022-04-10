/* Copyright (C) 2022    Joshua Noeske

    This program is free software: you can redistribute it and/or modify
    it under the terms of the GNU General Public License as published by
    the Free Software Foundation, either version 3 of the License, or
    (at your option) any later version.

    This program is distributed in the hope that it will be useful,
    but WITHOUT ANY WARRANTY; without even the implied warranty of
    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
    GNU General Public License for more details.

    You should have received a copy of the GNU General Public License
    along with this program.  If not, see <https://www.gnu.org/licenses/>.
*/

use crate::ServerStatus::{Inactive, Running, Starting};
use async_process::Command as AsyncCommand;
use fluent_templates::fluent_bundle::types::FluentNumber;
use fluent_templates::fluent_bundle::FluentValue;
use fluent_templates::{static_loader, LanguageIdentifier, Loader};
use frankenstein::MessageEntityType::Bold;
use frankenstein::{
    AnswerCallbackQueryParamsBuilder, Api, CallbackQuery, EditMessageReplyMarkupParamsBuilder,
    GetUpdatesParamsBuilder, InlineKeyboardButtonBuilder, InlineKeyboardMarkupBuilder, Message,
    MessageEntityBuilder, ReplyMarkup, SendMessageParamsBuilder, TelegramApi,
};
use futures_lite::io::BufReader;
use futures_lite::{AsyncBufReadExt, StreamExt};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::str::FromStr;
use std::string::String;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;
use std::{fs, str};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::sleep;

type ChatbridgeMap = Arc<RwLock<HashMap<String, JoinHandle<()>>>>;
type EnableChatbridgeAfterStartMap = Arc<RwLock<HashMap<String, Message>>>;

static_loader! {
    static LOCALES = {
        // The directory of localisations and fluent resources.
        locales: "./locales",
        // The language to falback on if something is not present.
        fallback_language: "en-UK"
    };
}

#[tokio::main]
async fn main() {
    // Read configuration json
    let config_file = fs::read_to_string("bot-config.json").expect("Error reading config file");
    let config: Config =
        serde_json::from_str(&config_file).expect("Could not parse config file. Aborting.");
    let token = config.token.as_str();
    println!("Configs (incl. token) read successfully");

    // Construct api
    let api = Api::new(token);

    //let bot_name = api.get_me().await.unwrap().result.username.unwrap();

    let mut update_params_builder = GetUpdatesParamsBuilder::default();
    update_params_builder
        .allowed_updates(vec!["message".to_string(), "callback_query".to_string()]);

    let mut update_params = update_params_builder.build().unwrap();

    let bot_data = BotData {
        locale: LanguageIdentifier::from_str(&config.locale)
            .expect("Could not parse language identifier."),
        api,
        config,
        chatbridge_map: Arc::new(RwLock::new(HashMap::new())),
        enable_chatbridge_after_start_map: Arc::new(RwLock::new(HashMap::new())),
    };

    println!("Start update loop.");
    loop {
        let result = bot_data.api.get_updates(&update_params);

        match result {
            Ok(response) => {
                for update in response.result {
                    if let Some(message) = update.message {
                        if bot_data
                            .config
                            .chat_server_map
                            .contains_key(&message.chat.id.to_string())
                        {
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
                    } else if let Some(callback_query) = update.callback_query {
                        if callback_query.message.as_ref().is_some() {
                            if bot_data.config.chat_server_map.contains_key(
                                &callback_query.message.as_ref().unwrap().chat.id.to_string(),
                            ) {
                                println!(
                                    "Callback query received from {:}, handling enabled.",
                                    callback_query.message.as_ref().unwrap().chat.id
                                );
                                let mut bot_data_clone = bot_data.clone();

                                tokio::spawn(async move {
                                    bot_data_clone.process_callback_query(callback_query).await;
                                });
                            } else {
                                println!(
                                    "Callback query received from {:}, no handling enabled.",
                                    callback_query.message.as_ref().unwrap().chat.id
                                );
                            }
                        } else {
                            println!(
                                "Callback query received from unknown sender, no handling enabled.",
                            );
                        }
                    }
                    update_params = update_params_builder
                        .offset(update.update_id + 1)
                        .build()
                        .unwrap();
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
    config: Config,
    locale: LanguageIdentifier,
    chatbridge_map: ChatbridgeMap,
    enable_chatbridge_after_start_map: EnableChatbridgeAfterStartMap,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct Config {
    token: String,
    rcon_password: String,
    locale: String,
    chat_server_map: HashMap<String, String>,
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
            } else if text.starts_with("/licence") {
                self.licence_handler(message).await;
            } else {
                self.pass_message_to_chatbridge(message).await;
            }
        }
    }

    async fn process_callback_query(&mut self, callback_query: CallbackQuery) {
        if let Some(callback_data) = &callback_query.data {
            if callback_data == "inline_enable_chatbridge" {
                self.enable_chatbridge_inline_handler(callback_query).await;
            }
        }
    }

    async fn start_server_handler(&self, message: Message) {
        let server_name = self.config.chat_server_map[&message.chat.id.to_string()].as_str();
        match self.get_service_active(&message) {
            Inactive => {
                {
                    let inline_keyboard = InlineKeyboardMarkupBuilder::default()
                        .inline_keyboard(vec![vec![InlineKeyboardButtonBuilder::default()
                            .text(LOCALES.lookup(&self.locale, "activate-chatbridge-inline"))
                            .callback_data("inline_enable_chatbridge")
                            .build()
                            .unwrap()]])
                        .build()
                        .unwrap();
                    let send_message_params = SendMessageParamsBuilder::default()
                        .chat_id(message.chat.id)
                        .text(LOCALES.lookup(&self.locale, "start-server"))
                        .reply_to_message_id(message.message_id)
                        .reply_markup(ReplyMarkup::InlineKeyboardMarkup(inline_keyboard))
                        .build()
                        .unwrap();

                    if let Err(err) = self.api.send_message(&send_message_params) {
                        println!("Failed to send message: {:?}", err);
                    }
                }

                println!("Start server {:}.", server_name);
                let service_name = format!("minecraft-server@{:}.service", server_name);
                Command::new("sudo")
                    .args(["systemctl", "start", &service_name])
                    .spawn()
                    .expect("Error executing command");

                let message_clone = message.clone();
                let server_name_clone = String::from(server_name);

                let (tx, rx) = mpsc::channel();

                let mut bot_data = self.clone();
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
                            println!("Server {} started.", server_name_clone);
                            bot_data
                                .send_message_with_reply(
                                    &message_clone,
                                    &LOCALES.lookup(&bot_data.locale, "server-started-now"),
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

                    let message_chatbridge_option = bot_data
                        .enable_chatbridge_after_start_map
                        .write()
                        .await
                        .remove(&message_clone.chat.id.to_string());

                    if let Some(message_chatbridge) = message_chatbridge_option {
                        println!("Start thread to enable chatbridge handler from start_server.");
                        tokio::spawn(async move {
                            bot_data.enable_chatbridge_handler(message_chatbridge).await;
                        });
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
                    self.send_message_with_reply(
                        &message,
                        &LOCALES.lookup(&self.locale, "server-started-unknown"),
                    )
                    .await;
                }
                println!(
                    "Finishing handling of start_server. Server {} was started properly: {}",
                    server_name, server_done
                );
            }
            Starting => {
                println!("Server {:} already starting.", server_name);
                self.send_message_with_reply(
                    &message,
                    &LOCALES.lookup(&self.locale, "server-starting-already"),
                )
                .await;
            }
            ServerStatus::Running { .. } => {
                println!("Server {:} already running.", server_name);
                self.send_message_with_reply(
                    &message,
                    &LOCALES.lookup(&self.locale, "server-running-already"),
                )
                .await;
            }
        }
    }

    async fn stop_server_handler(&mut self, message: Message) {
        let server_name = self.config.chat_server_map[&message.chat.id.to_string()].as_str();

        match self.get_service_active(&message) {
            Inactive => {
                self.send_message_with_reply(
                    &message,
                    &LOCALES.lookup(&self.locale, "server-not-running"),
                )
                .await;
                println!("Server {:} not running, cannot stop.", server_name);
            }
            Starting => {
                self.send_message_with_reply(
                    &message,
                    &LOCALES.lookup(&self.locale, "server-starting-cannot-stop"),
                )
                .await;
                println!("Server {:} currently starting, cannot stop.", server_name);
            }
            ServerStatus::Running { .. } => {
                self.send_message_with_reply(
                    &message,
                    &LOCALES.lookup(&self.locale, "stop-server"),
                )
                .await;
                println!("Stop server {:}.", server_name);
                self.disable_chatbridge_handler(message.clone(), false)
                    .await;
                let server_name =
                    self.config.chat_server_map[&message.chat.id.to_string()].as_str();
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
                self.send_message_with_reply(
                    &message,
                    &LOCALES.lookup(&self.locale, "server-not-running"),
                )
                .await;
            }
            Starting => {
                self.send_message_with_reply(
                    &message,
                    &LOCALES.lookup(&self.locale, "server-starting"),
                )
                .await;
            }
            ServerStatus::Running {
                current_players,
                max_players,
                players,
            } => {
                let reply = LOCALES.lookup_with_args(&self.locale, "server-running", &{
                    let mut map = HashMap::new();
                    map.insert(
                        String::from("currentPlayers"),
                        FluentValue::Number(FluentNumber::from(
                            u16::from_str(&current_players).unwrap(),
                        )),
                    );
                    map.insert(
                        String::from("maxPlayers"),
                        FluentValue::Number(FluentNumber::from(
                            u16::from_str(&max_players).unwrap(),
                        )),
                    );
                    map.insert(
                        String::from("players"),
                        FluentValue::String(Cow::from(players)),
                    );
                    map
                });

                self.send_message_with_reply(&message, &reply).await;
            }
        }
    }

    async fn enable_chatbridge_inline_handler(&mut self, callback_query: CallbackQuery) {
        if let Some(message) = callback_query.message {
            {
                let inline_keyboard = InlineKeyboardMarkupBuilder::default()
                    .inline_keyboard(vec![vec![]])
                    .build()
                    .unwrap();
                let edit_message_params = EditMessageReplyMarkupParamsBuilder::default()
                    .chat_id(message.chat.id)
                    .message_id(message.message_id)
                    .reply_markup(inline_keyboard)
                    .build()
                    .unwrap();
                if let Err(err) = self.api.edit_message_reply_markup(&edit_message_params) {
                    println!("Failed to send answer_callback_reply: {:?}", err);
                }
            }
            self.enable_chatbridge_handler(message).await;

            let answer_callback_query = AnswerCallbackQueryParamsBuilder::default()
                .callback_query_id(&callback_query.id)
                .build()
                .unwrap();
            if let Err(err) = self.api.answer_callback_query(&answer_callback_query) {
                println!("Failed to send answer_callback_reply: {:?}", err);
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
            self.send_message_with_reply(
                &message,
                &LOCALES.lookup(&self.locale, "chatbridge-activated"),
            )
            .await;
        } else {
            match self.get_service_active(&message) {
                Inactive => {
                    self.send_message_with_reply(
                        &message,
                        &LOCALES.lookup(
                            &self.locale,
                            "chatbridge-activation-not-possible-server-not-running",
                        ),
                    )
                    .await;
                }
                Starting => {
                    if !self
                        .enable_chatbridge_after_start_map
                        .read()
                        .await
                        .contains_key(&message.chat.id.to_string())
                    {
                        //TODO: This is not 100% thread-safe. Maybe change RwLock to Mutex and/or lock (write) for the whole Starting-scope.
                        self.send_message_with_reply(
                            &message,
                            &LOCALES.lookup(&self.locale, "activate-chatbridge-after-start"),
                        )
                        .await;
                        println!(
                            "Chat bridge will be activated for {} once the server is started.",
                            &message.chat.id.to_string()
                        );
                        self.enable_chatbridge_after_start_map
                            .write()
                            .await
                            .insert(message.chat.id.to_string(), message);
                    } else {
                        self.send_message_with_reply(
                            &message,
                            &LOCALES.lookup(&self.locale, "chatbridge-activation-already-prepared"),
                        )
                        .await;
                        println!(
                            "Chat bridge activation already prepared for {}.",
                            &message.chat.id.to_string()
                        );
                    }
                }
                ServerStatus::Running { .. } => {
                    self.send_message_with_reply(
                        &message,
                        &LOCALES.lookup(&self.locale, "activate-chatbridge"),
                    )
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
                            bot_data.config.chat_server_map[&message.chat.id.to_string()].as_str()
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
                }
            }
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
                self.send_message_with_reply(
                    &message,
                    &LOCALES.lookup(&self.locale, "chatbridge-deactivated"),
                )
                .await;
            }
        } else {
            let mut chatbridge_lock = self.chatbridge_map.write().await;
            if chatbridge_lock.contains_key(&message.chat.id.to_string()) {
                if send_message {
                    self.send_message_with_reply(
                        &message,
                        &LOCALES.lookup(&self.locale, "deactivate-chatbridge"),
                    )
                    .await;
                }
                println!(
                    "Chat bridge for {} gets deactivated.",
                    &message.chat.id.to_string()
                );
                chatbridge_lock[&message.chat.id.to_string()].abort();
                chatbridge_lock.remove(&message.chat.id.to_string());
            }
        }
    }

    async fn licence_handler(&self, message: Message) {
        self.send_message_with_reply(&message, &LOCALES.lookup(&self.locale, "licence"))
            .await;
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
                    self.config.rcon_password.as_str(),
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
        let server_name = self.config.chat_server_map[&message.chat.id.to_string()].as_str();
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
                    self.config.rcon_password.as_str(),
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
