use std::fmt::{Display, Formatter};
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use uds::tokio::{UnixSeqpacketConn};

use crate::connection::Server;
use crate::fs::TempUnixSeqpacketListener;

#[derive(Clone, Debug, Eq, PartialEq)]
enum ParseError {
    UnexpectedEnd,
    UnrecognizedOpcode(u8),
}

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        match self {
            Self::UnexpectedEnd => {
                "unexpected end".fmt(f)
            },
            Self::UnrecognizedOpcode(code) => {
                format!("unrecognized ocpode {}", code).fmt(f)
            },
        }
    }
}
impl std::error::Error for ParseError {}

#[derive(Clone, Debug, Eq, PartialEq)]
enum CommandBody {
    Register,
    Deregister,
}

impl CommandBody {
    pub fn parse(buf: &[u8]) -> Result<Self, ParseError> {
        if buf.len() < 1 {
            return Err(ParseError::UnexpectedEnd);
        }

        let opcode = buf[0];

        match opcode {
            1 => Ok(Self::Register),
            2 => Ok(Self::Deregister),
            _ => Err(ParseError::UnrecognizedOpcode(opcode)),
        }
    }
}

#[derive(Debug)]
pub struct Command {
    body: CommandBody,
    reply_tx: oneshot::Sender<Reply>,
}

impl Command {
    pub fn reply(self, reply: Reply) -> std::result::Result<(), Reply> {
        self.reply_tx.send(reply)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reply {
    Ok,
    MalformedCommand,
    Disconnected,
}

impl Reply {
    pub fn as_bytes(&self) -> &'static [u8] {
        match self {
            Reply::Ok => &[0],
            Reply::MalformedCommand => &[1],
            Reply::Disconnected => &[2],
        }
    }

    /// Returns true if this reply should end client communications.
    fn is_fatal(&self) -> bool {
        match self {
            Reply::Ok => false,
            Reply::MalformedCommand => false,

            Reply::Disconnected => true,
        }
    }
}

// TODO: maybe implement set class
// Could parse class

/// Handle a command received from a connected client.
async fn handle_client_command(
    buf: &[u8],
    command_tx: &mpsc::Sender<Command>,
) -> Reply {
    // Parse the command
    let command_body = match CommandBody::parse(buf) {
        Ok(body) => body,
        Err(e) => {
            eprintln!("Unable to parse command: {}", e);
            // Reply with an error
            return Reply::MalformedCommand;
        }
    };
    // Attach connection ID and a reply channel to command
    let (reply_tx, reply_rx) = oneshot::channel();
    let command = Command {
        body: command_body,
        reply_tx,
    };
    // Send the command to the main loop
    if let Err(e) = command_tx.send(command).await {
        eprintln!("Unable to send to main: {}", e);
        return Reply::Disconnected;
    }
    // Receive a reply
    let reply = match reply_rx.await {
        Ok(reply) => reply,
        Err(e) => {
            eprintln!("Unable to receive a reply from main: {}", e);
            return Reply::Disconnected;
        }
    };
    // Return the reply
    reply
}

/// Handle a single client command connection.
async fn handle_connection(
    client: &mut UnixSeqpacketConn,
    command_tx: &mpsc::Sender<Command>,
) {
    const BUF_SIZE: usize = 4096;
    let mut buf = [0u8; BUF_SIZE];
    loop {
        tokio::select! {
            len_res = client.recv(&mut buf) => {
                let len = match len_res {
                    Ok(len) => len,
                    Err(e) => {
                        // Exit.
                        eprintln!("Error receiving from client: {}", e);
                        break;
                    }
                };
                // Len == 0 indicates a disconnect.
                if len == 0 {
                    eprintln!("Disconnected from client");
                    break;
                }
                // Process connection
                let reply = handle_client_command(
                    &buf[..len],
                    command_tx).await;
                // Serialize and send the reply to the client.
                if client.send(reply.as_bytes()).await.is_err() {
                    eprintln!("Unable to reply to client");
                    break;
                }
                // Disconnect on a fatal error.
                if reply.is_fatal() {
                    break;
                }
            },
        }
    }
}


pub struct CommandServer {
    // Handles to tasks that process command connections.
    client_handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
    listener_handle: JoinHandle<()>,
}

impl CommandServer {

    /// Listen on the given socket and send commands from clients through the given mpsc.
    pub fn listen(
        mut command_listener: TempUnixSeqpacketListener,
        command_tx: mpsc::Sender<Command>,
    ) -> Self {
        let client_handles = Arc::new(Mutex::new(Vec::new()));

        // Create a movable clone of client_handles
        let listener_client_handles = Arc::clone(&client_handles);
        let listener_handle = tokio::spawn(async move {
            let mut next_connection_id = 0;

            loop {
                // Accept a connection from command_listener.
                let (mut connection, _) = match command_listener.accept().await {
                    Ok((connection, addr)) => (connection, addr),
                    Err(e) => {
                        eprintln!("Unable to accept connection: {}", e);
                        break;
                    }
                };

                // Assign the new connection an ID.
                let connection_id = next_connection_id;
                next_connection_id += 1;

                // Spawn a task to handle the connection.
                let client_command_tx = command_tx.clone();
                // Spawn a new task to handle a command connection
                let new_connection = tokio::spawn(async move {
                    handle_connection(
                        &mut connection,
                        &client_command_tx).await;
                    // Cleanup by deregistering
                    println!("Deregistering {}", connection_id);
                    let (reply_tx, reply_rx) = oneshot::channel();
                    let _ = client_command_tx.send(Command {
                        body: CommandBody::Deregister,
                        reply_tx,
                    }).await;
                    // Wait for but drop the reply.
                    let _ = reply_rx.await;
                });

                listener_client_handles.lock().unwrap().push(new_connection);
            }
        });

        Self {
            listener_handle,
            client_handles,
        }
    }

    /// Handle a single command sent by a client-handler task.
    pub async fn handle_command(
        &self,
        command: &Command,
        server: &mut Server,
    ) -> Reply {
        match &command.body {
            CommandBody::Register => {
                server.up().await.unwrap();
                Reply::Ok
            },
            CommandBody::Deregister => {
                server.down().await.unwrap();
                Reply::Ok
            },
        }
    }
}

impl Drop for CommandServer {
    fn drop(&mut self) {
        // Abort connections to clients
        for client_handle in self.client_handles.lock().unwrap().iter() {
            client_handle.abort();
        }
        // Abort the listening task.
        self.listener_handle.abort();
    }
}
