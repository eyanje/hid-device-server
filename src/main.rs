use bluer::Address;
use std::io::Read;
use tokio::sync::{broadcast, mpsc};

mod command;
mod connection;
mod event;
mod fs;
mod info_file;
mod pipe;
mod registration;

use command::CommandServer;
use connection::Server;
use event::{ServerEvent, EventServer};
use fs::TempUnixSeqpacketListener;

// Events

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    /*
    // Change device ID.
    // Need to use btmgmt because there is no equivalent HCI command.
    println!("Setting device ID");
    let client = Client::open().unwrap();
    let device_id_result = client.call(Some(0), SetDeviceId {
        source: DeviceIdSource::UsbImplementersForum,
        vendor: 0x057e,
        product: 0x0306,
        version: 0x0600,
    }).await;
    if let Err(e) = device_id_result {
        eprintln!("While setting device ID: {}", e);
    }
    */
    
    // TODO: rather than read fron stdin, receive signals in order to stop
    // (Or however systemd does it)

    // Spawn a task to read from stdin.
    let (user_input_tx, mut user_input_rx) = mpsc::channel::<()>(16);
    let user_input_handle = tokio::spawn(async move {
        let _tx = user_input_tx; // move
        let mut stdin = std::io::stdin();
        // Rather than quit externally, we should put REPL code in this thread and send the
        // termination signal out.
        loop {
            let mut b = [0u8];
            match stdin.read(&mut b) {
                Ok(_) => {
                    // Terminate the program.
                    break;
                },
                Err(_) => {
                    // Also terminate the program.
                    break;
                },
            }
        }
        println!("main: exited user input loop");
    });

    let event_listener = TempUnixSeqpacketListener::bind("event").unwrap();
    let (server_event_tx, mut server_event_rx) = broadcast::channel(16);
    let event_server = EventServer::listen(event_listener, server_event_tx.clone());

    // Start the server, which holds remote connections.
    let mut server = Server::new(Address::any(), server_event_tx.clone());

    // Receive commands from applications
    let command_listener = TempUnixSeqpacketListener::bind("command").unwrap();
    let (command_tx, mut command_rx) = mpsc::channel(16);
    let command_server = CommandServer::listen(command_listener, command_tx);

    loop {
        tokio::select! {
            command_res = command_rx.recv() => {
                let command = match command_res {
                    Some(command) => command,
                    None => {
                        eprintln!("Error receiving command to global");
                        continue;
                    },
                };
                // Consider moving this logic into main
                let reply = command_server.handle_command(
                    &command,
                    &mut server).await;
                // Return reply
                if let Err(r) = command.reply(reply) {
                    eprintln!("Error replying {:?} to command", r);
                    continue;
                }
            },
            // Listen for events
            server_event_res = server_event_rx.recv() => {
                let server_event = match server_event_res {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        ServerEvent::Lagged(skipped)
                    },
                    Err(e) => {
                        eprintln!("Error receiving event: {}", e);
                        break;
                    },
                };
                println!("Server event: {:?}", server_event);
            },
            _ = user_input_rx.recv() => {
                break;
            },
        }
    }
    
    drop(command_server);
    eprintln!("main: drop command_server");

    server.down().await.unwrap();
    eprintln!("main: server down");
    server.clear().await;
    eprintln!("main: server cleared");

    drop(event_server);
    eprintln!("main: event server down");

    user_input_handle.abort();
    eprintln!("main: input handle aborted");
}




