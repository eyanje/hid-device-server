use bluer::l2cap::{SeqPacket};
use libc::{EMSGSIZE, ENOTCONN};
use std::io::{Error, Result};
use std::mem;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::net::UnixDatagram;
use uds::nonblocking;

#[derive(Clone, Copy, Debug, Default)]
struct Swap<T> {
    pub main: T,
    pub swap: T,
}

impl <T> Swap<T> {
    pub fn new(main: T, swap: T) -> Self {
        Self { main, swap }
    }

    // Swaps the contents of this swap.
    pub fn swap(&mut self) {
        mem::swap(&mut self.main, &mut self.swap);
    }
}

// Copied from uds::tokio::UnixSeqpacketConn;

pub struct UnixSeqpacketConn {
    io: AsyncFd<nonblocking::UnixSeqpacketConn>,
}

impl UnixSeqpacketConn {
    /// Creates a tokio-compatible socket from an existing nonblocking socket.
    pub fn from_nonblocking(conn: nonblocking::UnixSeqpacketConn) -> Result<Self> {
        match AsyncFd::new(conn) {
            Ok(io) => Ok(Self { io }),
            Err(e) => Err(e),
        }
    }

    /// Deregisters the connection and returns the underlying non-blocking type.
    #[allow(dead_code)]
    pub fn into_nonblocking(self) -> nonblocking::UnixSeqpacketConn {
        self.io.into_inner()
    }

    /// Sends a packet to the socket's peer.
    pub async fn send(&self, packet: &[u8]) -> Result<usize> {
        self.io.async_io(Interest::WRITABLE, |conn| conn.send(packet)).await
    }
    /// Receives a packet from the socket's peer.
    pub async fn recv(&self, buffer: &mut[u8]) -> Result<usize> {
        self.io.async_io(Interest::READABLE, |conn| conn.recv(buffer)).await
    }
}

// End copied section

enum PipeLocal<'a> {
    Datagram(&'a UnixDatagram),
    Seqpacket(&'a UnixSeqpacketConn),
}

impl<'a> PipeLocal<'a> {
    pub async fn send(&self, packet: &[u8]) -> Result<usize> {
        match self {
            Self::Datagram(inner) => inner.send(packet).await,
            Self::Seqpacket(inner) => inner.send(packet).await,
        }
    }

    pub async fn recv(&self, buffer: &mut[u8]) -> Result<usize> {
        match self {
            Self::Datagram(inner) => inner.recv(buffer).await,
            Self::Seqpacket(inner) => inner.recv(buffer).await,
        }
    }
}


#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FromPipe<T> {
    FromRemote(T),
    FromLocal(T),
}

impl <T, E> FromPipe<std::result::Result<T, E>> {
    /// Convert a FromPipe of a Result into a FromPipe of an error, if the result was an error.
    pub fn into_result_error(self) -> Option<FromPipe<E>> {
        match self {
            Self::FromRemote(Err(e)) => Some(FromPipe::FromRemote(e)),
            Self::FromLocal(Err(e)) => Some(FromPipe::FromLocal(e)),
            _ => None,
        }
    }
}

pub struct Pipe<'a> {
    remote: &'a SeqPacket,
    local: PipeLocal<'a>,
    remote_to_local_buf: Swap<Vec<u8>>,
    remote_to_local_len: usize,
    local_to_remote_buf: Swap<Vec<u8>>,
    local_to_remote_len: usize,
}

const BUF_SIZE: usize = 1024;

impl <'a> Pipe<'a> {
    fn new(
        remote: &'a SeqPacket,
        local: PipeLocal<'a>,
    ) -> Self {
        Self {
            remote,
            local,
            remote_to_local_buf: Swap::new(
                [0u8].repeat(BUF_SIZE),
                [0u8].repeat(BUF_SIZE)),
            remote_to_local_len: 0,
            local_to_remote_buf: Swap::new(
                [0u8].repeat(BUF_SIZE),
                [0u8].repeat(BUF_SIZE)),
            local_to_remote_len: 0,
        }
    }

    pub fn new_datagram(
        remote: &'a SeqPacket,
        local: &'a UnixDatagram,
    ) -> Self {
        Self::new(remote, PipeLocal::Datagram(local))
    }

    pub fn new_seqpacket(
        remote: &'a SeqPacket,
        local: &'a UnixSeqpacketConn,
    ) -> Self {
        Self::new(remote, PipeLocal::Seqpacket(local))
    }

    /// Remove any unsent message in the remote-to-local buffer.
    pub fn clear_remote_to_local_buf(&mut self) {
        self.remote_to_local_len = 0;
    }

    /// Remove any unsent message in the local-to-remote buffer.
    #[allow(dead_code)]
    pub fn clear_local_to_remote_buf(&mut self) {
        self.local_to_remote_len = 0;
    }

    pub async fn work(&mut self) -> FromPipe<Result<usize>> {
        // We use the main for reading and swap for writing.
        // We will also transpose local into a read and write half.

        tokio::select! {
            // Read from remote, if the main buffer is empty.
            len_res = self.remote.recv(&mut self.remote_to_local_buf.swap),
            if self.remote_to_local_len == 0 => {
                match len_res {
                    Ok(len) => {
                        // Sometimes, on disconnect, we continue to receive zero-length messages.
                        if len == 0 {
                            return FromPipe::FromRemote(Err(Error::from_raw_os_error(ENOTCONN)));
                        }

                        // Swap data to main
                        self.remote_to_local_buf.swap();
                        // Save data length
                        self.remote_to_local_len = len;
                        FromPipe::FromRemote(Ok(len))
                    },
                    Err(e) => {
                        FromPipe::FromRemote(Err(e))
                    }
                }
            },
            // Write to local, if the buffer has contents.
            len_res = self.local.send(
                &self.remote_to_local_buf.main[..self.remote_to_local_len]
            ), if self.remote_to_local_len > 0 => {
                match len_res {
                    Ok(len) => {
                        // If we didn't complete the write, return an error.
                        if len < self.remote_to_local_len {
                            eprintln!("Partial write to local of len {} out of {}",
                                      len, self.remote_to_local_len);
                            FromPipe::FromLocal(Err(Error::from_raw_os_error(EMSGSIZE)))
                        } else {
                            // If the write succeeded, reset the length.
                            self.remote_to_local_len = 0;
                            FromPipe::FromLocal(Ok(len))
                        }
                    },
                    Err(e) => {
                        FromPipe::FromLocal(Err(e))
                    },
                }
            },
            // Read from local, if the buffer is empty.
            len_res = self.local.recv(&mut self.local_to_remote_buf.swap),
            if self.local_to_remote_len == 0 => {
                match len_res {
                    Ok(len) => {
                        if len == 0 {
                            return FromPipe::FromLocal(Err(Error::from_raw_os_error(ENOTCONN)));
                        }
                        // Swap data to main
                        self.local_to_remote_buf.swap();
                        // Save data length
                        self.local_to_remote_len = len;
                        FromPipe::FromLocal(Ok(len))
                    },
                    Err(e) => {
                        FromPipe::FromLocal(Err(e))
                    }
                }
            },
            // Write to remote, if the buffer has contents.
            len_res = self.remote.send(
                &self.local_to_remote_buf.main[..self.local_to_remote_len]
            ), if self.local_to_remote_len > 0 => {
                match len_res {
                    Ok(len) => {
                        /*println!("Written to remote: {:02X?}",
                                 &self.local_to_remote_buf.main[..len]);*/
                        // If we didn't complete the write, return an error.
                        if len < self.local_to_remote_len {
                            eprintln!("Partial write to remote of len {} out of {}",
                                      len, self.local_to_remote_len);
                            FromPipe::FromRemote(Err(Error::from_raw_os_error(EMSGSIZE)))
                        } else {
                            // If the write succeeded, reset the length.
                            self.local_to_remote_len = 0;
                            FromPipe::FromRemote(Ok(len))
                        }
                    },
                    Err(e) => {
                        FromPipe::FromRemote(Err(e))
                    },
                }
            },
        }
    }
    
    pub async fn work_until_error(&mut self) -> FromPipe<Error> {
        loop {
            if let Some(e) = self.work().await.into_result_error() {
                return e;
            }
        }
    }
}


