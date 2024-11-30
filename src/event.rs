use bluer::Address;
use std::iter::once;
use std::net::Shutdown;
use std::sync::{Arc, Mutex};
use tokio::sync::{broadcast};
use tokio::task::JoinHandle;
use uds::tokio::UnixSeqpacketConn;

use crate::fs::TempUnixSeqpacketListener;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ServerEvent {
    Lagged(u64),
    ControlListening(Address),
    InterruptListening(Address),
    Disconnected(Address),
}

impl ServerEvent {
    /// Return the opcode of this event.
    pub fn code(&self) -> u8 {
        match self {
            Self::Lagged(..) => 0x01,
            Self::ControlListening(..) => 0x02,
            Self::InterruptListening(..) => 0x03,
            Self::Disconnected(..) => 0x04,
        }
    }

    /// Convert this event to a Vec of bytes
    pub fn as_bytes(&self) -> Vec<u8> {
        match self {
            Self::Lagged(skipped) => {
                once(self.code())
                    .chain(skipped.to_le_bytes().into_iter())
                    .collect()
            },
            Self::ControlListening(address) |
            Self::InterruptListening(address) |
            Self::Disconnected(address) => {
                once(self.code())
                    .chain(address.0.into_iter())
                    .collect()
            },
        }
    }
}

async fn handle_connection(
    connection: &mut UnixSeqpacketConn,
    mut events_rx: broadcast::Receiver<ServerEvent>,
) {
    // Shut down the read half
    if let Err(e) = connection.shutdown(Shutdown::Read) {
        eprintln!("Error shutting down event reads: {}", e);
        return;
    }

    loop {
        let event = match events_rx.recv().await {
            Ok(e) => e,
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                ServerEvent::Lagged(skipped)
            },
            Err(e) => {
                eprintln!("Error receiving event: {}", e);
                break;
            },
        };
        // Serialize and send the event to the socket.
        if let Err(e) = connection.send(&event.as_bytes()).await {
            eprintln!("Error sending event to client: {}", e);
            break;
        }
    }

    eprintln!("event::handle_connection: disconnected");
}


pub struct EventServer {
    listener_handle: JoinHandle<()>,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl EventServer {
    pub fn listen(
        mut event_listener: TempUnixSeqpacketListener,
        server_event_tx: broadcast::Sender<ServerEvent>,
    ) -> Self {
        let connections = Arc::new(Mutex::new(Vec::new()));

        // Start the event server
        let listener_connections = Arc::clone(&connections);
        let listener_handle = tokio::spawn(async move {
            loop {
                let mut new_connection = match event_listener.accept().await {
                    Ok((conn, _addr)) => conn,
                    Err(e) => {
                        eprintln!("Error accepting event client: {}", e);
                        break;
                    },
                };
                let new_server_event_rx = server_event_tx.subscribe();
                let connection_handle = tokio::spawn(async move {
                    handle_connection(
                        &mut new_connection,
                        new_server_event_rx
                    ).await;
                });
                listener_connections.lock().unwrap().push(connection_handle);
            }
        });

        Self {
            listener_handle,
            connections,
        }
    }
}

impl Drop for EventServer {
    fn drop(&mut self) {
        self.listener_handle.abort();
        for connection in self.connections.lock().unwrap().drain(..) {
            connection.abort();
        }
    }
}
