activate-chatbridge-inline = Activate chat bridge
start-server = I start the server. If you want to activate the chat bridge, press the button beneath the message.
server-started-now = The server is running now.
server-started-unknown = The server was started, however it cannot be determined if it is running now. This is usually an indication that the server encountered an error during its start. Please contact your admin to ask what went wrong.
server-starting-already = The server is already starting.
server-running-already = The server is running already.
server-not-running = The server is not running currently.
server-starting = The server is starting currently.
server-starting-cannot-stop = The server is currently starting. Please wait until the server is done with starting before you shut it down.
stop-server = I stop the server.
server-stopped-externally = The server was stopped by another chat or an external system. I deactivate the chat bridge.
server-running =
    { $currentPlayers ->
        [0] The server is running. However, nobody is online right now.
        *[other] The server is running and there are { $currentPlayers } of { $maxPlayers } players online: { $players }
    }
chatbridge-activated = The chat bridge is already activated.
chatbridge-activation-not-possible-server-not-running = The server is not running currently. Therefore, you cannot start the chat bridge.
activate-chatbridge-after-start = Ok! When the server is done starting, I'll activate the chat bridge.
chatbridge-activation-already-prepared = The activation of the chat bridge is already prepared.
activate-chatbridge = I activate the chat bridge.
chatbridge-deactivated = The chat bridge is already activated.
deactivate-chatbridge = I deactivate the chat bridge.
licence = This bot is free and libre software! The source code is licenced under the terms of the GPLv3 or any later version. The source code is available at https://github.com/PatrickJosh/minecraft-server-telegram-bot.
