# Minecraft Server Telegram Bot
This software can be used to start/stop Minecraft servers via systemd and get the current status
of servers, all via a Telegram bot.
It can also mirror the chat messages in Minecraft to a Telegram chat and vice versa.

To be able to use the software, one has to have a setup similar to the one in explained
[here](https://github.com/PatrickJosh/minecraft-server-systemd-service). Most importantly, the start and stop commands
have to have the same structure as mentioned in the explanation. If this is not the case, then to have to edit the
command syntax in the sources of this software.

## Configuration
The setup is written as a bit of a follow-up to the explanation given in
[this repository](https://github.com/PatrickJosh/minecraft-server-systemd-service).
Especially the steps 1 to 3 of the section “Setting up the systemd-service” are necessary.

### Compile the project
Just use `cargo build --release`.
The constructed binary can be found at `target/release/minecraft-server-telegram-bot`.

### Setting up the bot
1. Obtain a token from the [BotFather](https://t.me/BotFather). In the following, it is referred to as `<token>`.
2. Create a new folder `/var/minecraft/telegram-bot`.
3. Copy the produced binary as well as the `bot-config.json` to that folder and set the rights properly:
```shell
# chown -R root:root /var/minecraft/telegram-bot
# chmod 755 /var/minecraft/telegram-bot/minecraft-server-telegram-bot
# semanage fcontext -a -f bin_t '/var/minecraft/telegram-bot/minecraft-server-telegram-bot'
# restorecon -v /var/minecraft/telegram-bot/minecraft-server-telegram-bot
```
4. Open the `bot-config.json` and edit the configuration as follows:
    1. Enter the obtained token.
    2. Enter the RCON password used for your servers.
    3. Use the `chat_server_map` to set which chats may control which servers. Enter the chat id on the left, the server
       name on the right. It must be an n:1 relation, so one chat may control one server, but one server may be controlled
       by many chats.
       To see how a chat id for a particular chat can be obtained, see
       [here](https://stackoverflow.com/questions/32423837/telegram-bot-how-to-get-a-group-chat-id#32572159).

Now you can run the server by using
```shell
$ sudo -u minecraft /var/minecraft/telegram-bot/minecraft-server-telegram-bot
```

You can also install a systemd-service for the bot by copying `systemd-service/minecraft-telegram-bot.service` to
`/etc/systemd/system` and executing
```shell
# systemctl daemon-reload
```

## Known issues
- Currently, the messages sent by the bot are in German only (as it is my mother tongue and this started as a small
  personal project).
  For more on that, see issue #2.
- The project currently lacks proper documentation.

## Contribution
I am happy about any contribution you want to make to this project! If you want to do any major contribution, please
open an issue before submitting a pull request, so we can coordinate it (so no work is done twice).

## Licencing
The software is licenced under the terms of the GNU General Public License, Version 3 or later.

Copyright (C) 2022 Joshua Noeske

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