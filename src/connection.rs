use bluer::{Address, AddressType};
use bluer::l2cap::{Socket, SocketAddr, SeqPacket, SeqPacketListener};
use hid_device_id::bluetooth::psm;
use libc::{EISCONN, ENOTCONN};
use std::fmt::{Display, Formatter};
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::{broadcast, mpsc};
use tokio::task::{JoinError, JoinHandle};

use crate::event::{ServerEvent};
use crate::fs::{FileExt, TempDir, TempFile, TempUnixDatagram, TempUnixSeqpacketListener};
use crate::pipe::{self, FromPipe, Pipe};


/*
/// Events sent to a connection
pub enum ConnectionEvent {
    /// Single-time event for when an interrupt channel is opened.
    /// Not sure if only one interrupt connection ever exists, but the main loop only accepts one.
    InterruptEstablished(SeqPacket),
}
*/    
    

/// Struct for working with a control socket.
struct LocalControlSocket {
    local_control_listener: TempUnixSeqpacketListener,
}

impl LocalControlSocket {
    pub fn bind<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Ok(Self {
            local_control_listener: TempUnixSeqpacketListener::bind(&path)?,
        })
    }

    /// When a connection is received from the local listener, begin transferring data with the socket.
    pub async fn handle_control_socket(
        &mut self,
        peer_address: Address,
        remote_control_socket: &SeqPacket,
        server_event_tx: broadcast::Sender<ServerEvent>,
    ) {
        loop {
            // Broadcast an event, ignoring errors.
            let _ = server_event_tx.send(ServerEvent::ControlListening(peer_address));

            // Accept a local control socket
            let local_control_socket = match self.local_control_listener.accept().await {
                Ok((sock, _addr)) => sock,
                Err(e) => {
                    eprintln!("Error accepting local control: {}", e);
                    break;
                },
            };

            println!("Accepted local control connection");

            // Convert to and from nonblocking, for Pipe.
            let local_control_socket_nonblocking = local_control_socket.into_nonblocking();
            let local_control_socket_pipe = match pipe::UnixSeqpacketConn::from_nonblocking(
                    local_control_socket_nonblocking) {
                Ok(sock) => sock,
                Err(e) => {
                    eprintln!("Error creating tokio control socket: {}", e);
                    break;
                },
            };

            let mut pipe = Pipe::new_seqpacket(
                &remote_control_socket,
                &local_control_socket_pipe);

            match pipe.work_until_error().await {
                // If the remote side shuts down, exit.
                FromPipe::FromRemote(e) => {
                    eprintln!("Error from remote control: {}", e);
                    break;
                },
                // If the local side shuts down, wait to reconnect.
                FromPipe::FromLocal(e) => {
                    eprintln!("Error from local control: {}", e);
                    continue;
                },
            }
        }
    }
}


/// Struct for handling an interrupt socket
struct LocalInterruptSocket {
    local_interrupt_socket: TempUnixDatagram,
}

impl LocalInterruptSocket {
    pub fn bind<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        Ok(Self {
            local_interrupt_socket: TempUnixDatagram::bind(&path)?,
        })
    }

    /// When the interrupt socket is established and a connection is received from the local listener,
    /// begin transferring interrupt data.
    async fn handle_interrupt_socket(
        &mut self,
        peer_address: Address,
        interrupt_rx: mpsc::Receiver<SeqPacket>,
        ready_file: TempFile,
        server_event_tx: broadcast::Sender<ServerEvent>,
    ) {
        // Receive remote interrupt socket.
        let remote_interrupt_socket = {
            let mut interrupt_rx_moved = interrupt_rx;
            match interrupt_rx_moved.recv().await {
                Some(socket) => socket,
                None => {
                    eprintln!("Failed to receive interrupt socket.");
                    return;
                },
            }
        };
        eprintln!("Received interrupt socket.");
        
        // Update the ready file
        if let Err(e) = ready_file.write_to_start(b"1") {
            eprintln!("Unable to create ready file: {}", e);
            return;
        }

        // Connect to loal interrupt socket.
        loop {
            // Broadcast an event, ignoring errors.
            let _ = server_event_tx.send(ServerEvent::InterruptListening(peer_address));

            // Connect the local and remote interrupt sockets with a Pipe.
            let mut pipe = Pipe::new_datagram(
                &remote_interrupt_socket,
                &self.local_interrupt_socket);

            // Let the pipe work until error.
            match pipe.work_until_error().await {
                // If the remote side shuts down, exit.
                FromPipe::FromRemote(e) => {
                    eprintln!("Error from remote interrupt: {}", e);
                    break;
                },
                // If the local side shuts down, clear the last message to local and restart.
                FromPipe::FromLocal(e) => {
                    eprintln!("Error from local interrupt: {}", e);
                    pipe.clear_remote_to_local_buf();
                    continue;
                },
            }
        }
    }
}


#[derive(Debug)]
pub struct CancelHandle<T> {
    join_handle: JoinHandle<T>,
    cancel_tx: mpsc::Sender<()>,
}

impl<T> CancelHandle<T> {
    /// Returns true if the task has finished.
    pub fn is_finished(&self) -> bool {
        self.join_handle.is_finished()
    }

    /// Cleanly cancel this CancelHandle.
    /// Panics if called twice.
    pub async fn cancel(&mut self) -> <Self as Future>::Output {
        // Send a message through the cancel handle
        // If the task has already been cancelled, this should eventually error.
        let _ = self.cancel_tx.send(()).await;
        // Wait for completion
        (&mut self.join_handle).await
    }
}

// Allow a CancelHandle to be awaited, just like JoinHandle.
impl<T> Future for CancelHandle<T> {
    type Output = <JoinHandle::<T> as Future>::Output;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { self.map_unchecked_mut(|s| &mut s.join_handle) }.poll(cx)
    }
}

// Don't allow a CancelHandle to drop when running.
impl<T> Drop for CancelHandle<T> {
    fn drop(&mut self) {
        if !self.join_handle.is_finished() {
            panic!("CancelHandle unfinished");
        }
    }
}


#[derive(Debug)]
enum HandleConnectionError {
    CreatingConnectionDirectory(std::io::Error),
    CreatingInfoFile(std::io::Error),
    OpeningLocalControl(PathBuf, std::io::Error),
    OpeningLocalInterrupt(PathBuf, std::io::Error),
}

impl Display for HandleConnectionError {
    fn fmt(&self, f: &mut Formatter) -> std::fmt::Result {
        match self {
            Self::CreatingConnectionDirectory(e) =>
                write!(f, "while creating connection directory: {}", e),
            Self::CreatingInfoFile(e) =>
                write!(f, "while creating info file: {}", e),
            Self::OpeningLocalControl(path, e) =>
                write!(f, "while opening local control at {}: {}", path.display(), e),
            Self::OpeningLocalInterrupt(path, e) =>
                write!(f, "while opening local interrupt at {}: {}", path.display(), e),
        }
    }
}

/// Establish sockets and I/O loops for the control and interrupt sockets, as well as an event
/// socket.
fn handle_connection<P: AsRef<Path>>(
    peer_address: Address,
    runtime_directory: P,
    control_socket: SeqPacket,
    interrupt_rx: mpsc::Receiver<SeqPacket>,
    server_event_tx: broadcast::Sender<ServerEvent>,
) -> std::result::Result<CancelHandle<()>, HandleConnectionError> {
    // R/W for the group
    // Create directory for sockets
    let connection_dir_name = PathBuf::from(
        peer_address.to_string().replace(":", "_"));
    let connection_dir_path = runtime_directory.as_ref().join(connection_dir_name);
    let connection_dir = match TempDir::new(&connection_dir_path) {
        Ok(dir) => dir,
        Err(e) => {
            return Err(HandleConnectionError::CreatingConnectionDirectory(e));
        },
    };
    
    // Create informational files

    let ready_path = connection_dir_path.join("ready");
    let ready_file = TempFile::new_with_data(ready_path, b"0")
        .map_err(|e| HandleConnectionError::CreatingInfoFile(e))?;

    // Create local event, control, and interrupt sockets.
    // Listen on the event and control sockets.
    // let local_event_socket_path = connection_dir_path.join("event");
    let local_control_socket_path = connection_dir_path.join("control");
    let local_interrupt_socket_path = connection_dir_path.join("interrupt");
    
    let mut local_control_socket = match LocalControlSocket::bind(
        &local_control_socket_path
    ) {
        Ok(l) => l,
        Err(e) => {
            return Err(HandleConnectionError::OpeningLocalControl(
                    local_control_socket_path, e));
        },
    };
    let mut local_interrupt_socket = match LocalInterruptSocket::bind(
        &local_interrupt_socket_path
    ) {
        Ok(l) => l,
        Err(e) => {
            return Err(HandleConnectionError::OpeningLocalInterrupt(
                    local_interrupt_socket_path, e));
        },
    };

    // Spawn new abortable tasks to listen on the control and interrupt sockets.

    let control_server_event_tx = server_event_tx.clone();
    let mut control_handler = tokio::spawn(async move {
            local_control_socket.handle_control_socket(
            peer_address,
            &control_socket,
            control_server_event_tx,
        ).await;
    });

    let interrupt_server_event_tx = server_event_tx.clone();
    let mut interrupt_handler = tokio::spawn(async move {
        local_interrupt_socket.handle_interrupt_socket(
            peer_address,
            interrupt_rx,
            ready_file,
            interrupt_server_event_tx,
        ).await;
    });

    let (cancel_tx, mut cancel_rx) = mpsc::channel(1);

    // Single cancellable handler with control over the connection's life.
    let join_handle = tokio::spawn(async move {
        let mut control_awaited = false;
        let mut interrupt_awaited = false;
        // Wait for a handler or a cancellation
        tokio::select! {
            _ = &mut control_handler => {
                eprintln!("handle_connection ended from control");
                control_awaited = true;
            },
            _ = &mut interrupt_handler => {
                eprintln!("handle_connection ended from interrupt");
                interrupt_awaited = true;
            },
            _ = cancel_rx.recv() => {
                eprintln!("handle_connection cancelled");
            },
        }

        // Notify of disconnect
        let _ = server_event_tx.send(ServerEvent::Disconnected(peer_address));

        // Can't await handlers here because awaiting twice on a task panics.
        if !interrupt_awaited {
            interrupt_handler.abort();
            let _ = interrupt_handler.await;
        }
        if !control_awaited {
            control_handler.abort();
            let _ = control_handler.await;
        }

        // Drop directory.
        drop(connection_dir);
    });

    Ok(CancelHandle {
        join_handle,
        cancel_tx,
    })
}



#[derive(Debug)]
pub struct Connection {
    peer_address: Address,
    /// Handle to a call to handle_connection().
    connection_handle: CancelHandle<()>,
    interrupt_tx: mpsc::Sender<SeqPacket>,
}

impl Connection {
    pub fn peer_address(&self) -> Address {
        self.peer_address
    }

    /// Returns true if the connection has completed.
    pub fn is_finished(&self) -> bool {
        self.connection_handle.is_finished()
    }

    pub async fn cancel(&mut self) {
        eprintln!("connection: cancel {}", self.peer_address());
        let _ = self.connection_handle.cancel().await;
    }
}

/// Cancel an existing connection with the given address, if it can be found. If no such connection
/// exists, this function does nothing.
async fn cancel_existing(connections_mutex: &Mutex<Vec<Connection>>, peer_address: Address) {
    let existing_connection_opt = {
        let mut connections = connections_mutex.lock().unwrap();

        // Remove dead connections
        connections.retain(|conn| !conn.is_finished());

        // Cancel the old condition
        connections.iter()
            .position(|conn| conn.peer_address() == peer_address)
            .map(|pos| connections.remove(pos))
    };
    if let Some(mut existing_connection) = existing_connection_opt {
        eprintln!("cancel_existing: duplicate connection to {}", peer_address);
        // Ignore the result, which only indicates whether the connection finished cleanly or not.
        let _ = existing_connection.cancel().await;
    }
}


async fn receive_control_connection<P: AsRef<Path>>(
    connections_mutex: Arc<Mutex<Vec<Connection>>>,
    runtime_directory: P,
    control: SeqPacket,
    peer_address: Address,
    server_event_tx: broadcast::Sender<ServerEvent>,
) {
    // Delete duplicate connections.
    // handle_connection can get stuck waiting for a local connection on the control socket, which
    // means that the remote socket is never read. This can easily leave a ghost connection that
    // can never be deleted.
    cancel_existing(&connections_mutex, peer_address).await;

    let (interrupt_tx, interrupt_rx) = mpsc::channel(1);

    // Create a new connection with the peer
    let connection_handle = match handle_connection(
        peer_address,
        runtime_directory,
        control,
        interrupt_rx,
        server_event_tx.clone(),
    ) {
        Ok(handle) => handle,
        Err(e) => {
            eprintln!("Error handling connection to {}: {}", peer_address, e);
            return;
        }
    };
    
    // Try to save the connection
    // It isn't possible to hold two valid handles to handle_connection, unless the file system has
    // desynced. Therefore, if we reached this point, no additional connections have been created,
    // and added to the vec.

    let mut connections = connections_mutex.lock().unwrap();

    // Out of curiosity, check if somehow another connection has been created and added.
    if connections.iter().any(|conn| conn.peer_address() == peer_address) {
        panic!("Connection vec met impossible race condition");
    }

    let new_connection = Connection {
        peer_address: peer_address,
        connection_handle,
        interrupt_tx,
    };
    connections.push(new_connection);
}


async fn receive_interrupt_connection(
    connections_mutex: &Mutex<Vec<Connection>>,
    interrupt: SeqPacket,
    peer_address: Address,
) -> io::Result<()> {
    let existing_interrupt_tx_opt = {
        connections_mutex.lock().unwrap()
            .iter()
            .find(|c| c.peer_address == peer_address)
            .map(|c| c.interrupt_tx.clone())
    };

    if let Some(existing_interrupt_tx) = existing_interrupt_tx_opt {
        // Send an interrupt established event.
        // Ignore errors, such as if there is no receiver.
        if let Err(e) = existing_interrupt_tx.send(interrupt).await {
            eprintln!("Error sending interrupt: {}", e);
            return Err(io::Error::from_raw_os_error(EISCONN))
        }

        Ok(())
    } else {
        eprintln!("Dangling interrupt connection received from {}", peer_address);
        Err(io::Error::from_raw_os_error(ENOTCONN))
    }
}

struct ListeningServer {
    control_listener: SeqPacketListener,
    interrupt_listener: SeqPacketListener,
}

impl ListeningServer {
    /// Construct a ListeningServer bound to the given address.
    pub fn listen(address: Address) -> io::Result<Self> {
        let control_listener_socket = Socket::new_seq_packet()?;
        let interrupt_listener_socket = Socket::new_seq_packet()?;
        // Bind sockets to their respective addresses.
        control_listener_socket.bind(SocketAddr {
            addr: address,
            addr_type: AddressType::BrEdr,
            psm: psm::HID_CONTROL,
            cid: 0,
        })?;
        interrupt_listener_socket.bind(SocketAddr {
            addr: address,
            addr_type: AddressType::BrEdr,
            psm: psm::HID_INTERRUPT,
            cid: 0,
        })?;

        Ok(Self {
            control_listener: control_listener_socket.listen(0)?,
            interrupt_listener: interrupt_listener_socket.listen(0)?,
        })
    }


    /// Begin accepting connections.
    pub fn accept(
        self,
        runtime_directory: PathBuf,
        connections: Arc<Mutex<Vec<Connection>>>,
        server_event_tx: broadcast::Sender<ServerEvent>,
    ) -> AcceptingServer {
        AcceptingServer::accept(
            runtime_directory,
            self.control_listener,
            self.interrupt_listener,
            connections,
            server_event_tx)
    }
}

/// A struct representing a running accept loop
/// Can be dropped to stop both listening and accepting.
struct AcceptingServer {
    cancel_tx: mpsc::Sender<()>,
    accept_handle: JoinHandle<()>,
}

impl AcceptingServer {
    /// Begin accepting connections.
    pub fn accept(
        runtime_directory: PathBuf,
        control_listener: SeqPacketListener,
        interrupt_listener: SeqPacketListener,
        connections_mutex: Arc<Mutex<Vec<Connection>>>,
        server_event_tx: broadcast::Sender<ServerEvent>,
    ) -> Self {
        let (cancel_tx, mut cancel_rx) = mpsc::channel(1);
        let accept_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    control_result = control_listener.accept() => {
                        let (control, peer_addr) = match control_result {
                            Ok((control, peer_addr)) => (control, peer_addr),
                            // If we could not receive the control connection, exit.
                            Err(e) => {
                                eprintln!("Error receiving control connection: {}", e);
                                break;
                            }
                        };

                        // Register the new control connection
                        receive_control_connection(
                            connections_mutex.clone(),
                            &runtime_directory,
                            control,
                            peer_addr.addr,
                            server_event_tx.clone()).await;

                        println!("Accepted control connection from {}", peer_addr.addr);
                    },
                    interrupt_result = interrupt_listener.accept() => {
                        let (interrupt, peer_addr) = match interrupt_result {
                            Ok((interrupt, peer_addr)) => (interrupt, peer_addr),
                            // If we could not receive the interrupt connection, exit.
                            Err(e) => {
                                eprintln!("Error receiving interrupt connection: {}", e);
                                break;
                            }
                        };

                        // Register a new interrupt connection
                        if let Err(e) = receive_interrupt_connection(
                            &connections_mutex,
                            interrupt,
                            peer_addr.addr,
                        ).await {
                            eprintln!("While accepting interrupt connection to {}: {}",
                                      peer_addr.addr, e);
                            break;
                        }

                        println!("Accepted interrupt connection from {}", peer_addr.addr);
                    },
                    _ = cancel_rx.recv() => {
                        break;
                    },
                }
            }
        });

        Self {
            cancel_tx,
            accept_handle,
        }
    }

    /// Cleanly cancel this loop, if it is running.
    pub async fn cancel(&self) {
        let _ = self.cancel_tx.send(()).await;
    }

    /// Waits for the task to finish.
    ///
    /// This function will block forever if the task has not been stopped already.
    pub async fn join(&mut self) -> std::result::Result<(), JoinError> {
        (&mut self.accept_handle).await
    }
}

impl Drop for AcceptingServer {
    /// Immediately aborts the currently running loop, potenetially while accepting a connection.
    fn drop(&mut self) {
        self.accept_handle.abort();
    }
}


/// Persistent server of connections
pub struct Server {
    address: Address,
    runtime_directory: PathBuf,
    connections_mutex: Arc<Mutex<Vec<Connection>>>,
    accepting_server: Option<AcceptingServer>,
    server_event_tx: broadcast::Sender<ServerEvent>,
}

impl Server {
    pub fn new(address: Address,
               runtime_directory: PathBuf,
               server_event_tx: broadcast::Sender<ServerEvent>) -> Self {
        Self {
            address,
            runtime_directory,
            connections_mutex: Arc::new(Mutex::new(Vec::new())),
            accepting_server: None,
            server_event_tx,
        }
    }

    /// Open the server to incoming connections, using the given registration.
    pub async fn up(
        &mut self,
    ) -> std::io::Result<()> {
        // If there is currently a running handle, do nothing.
        // Might not catch all cases, but should catch enough.
        if self.accepting_server.is_some() {
            return Ok(());
        }
        // To be safe, try to listen for a connection.
        let listening_server = match ListeningServer::listen(self.address) {
            Ok(server) => server,
            Err(e) => {
                return Err(e);
            }
        };

        // Begin accepting connections
        self.accepting_server = Some(listening_server.accept(
                self.runtime_directory.clone(),
                self.connections_mutex.clone(),
                self.server_event_tx.clone()));
        Ok(())
    }

    pub async fn down(&mut self) -> std::result::Result<(), JoinError> {
        // Take the server down
        if let Some(server) = self.accepting_server.as_mut() {
            server.cancel().await;
            server.join().await?;
        }
        
        // Remove the server
        self.accepting_server = None;

        Ok(())
    }

    /// Cancels all current connections
    pub async fn clear(&mut self) {
        for mut conn in self.connections_mutex.lock().unwrap().drain(..) {
            conn.cancel().await;
        }
    }
}




