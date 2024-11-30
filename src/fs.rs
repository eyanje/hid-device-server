use std::fs::{create_dir, Permissions, remove_dir, remove_file, set_permissions, write};
use std::io::{Result};
use std::path::{Path, PathBuf};
use std::ops::{Deref, DerefMut};
use std::os::unix::fs::PermissionsExt;
use tokio::net::UnixDatagram;
use uds::tokio::UnixSeqpacketListener;

pub struct TempFile {
    path: PathBuf,
}

impl TempFile {
    /// Create a new TempFile at the given path.
    ///
    /// This function may fail if the path cannot be created.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        Ok(Self {
            path: path.as_ref().to_owned(),
        })
    }

    /// Returns a reference to the path of this file.
    pub fn path(&self) -> &Path {
        &self.path
    }

}

impl Drop for TempFile {
    /// Drop this TempFile by deleting it.
    fn drop(&mut self) {
        if let Err(e) = remove_file(self.path()) {
            eprintln!("while removing file at {}: {}", self.path().display(), e);
        }
    }
}

pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create a new TempDir at the given path.
    ///
    /// This function may fail if the path cannot be created.
    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        /*
        // Kind of a hack solution. Delete any existing directories, then create a new directory.
        // This prevents a file-exists error, but only if the directory is empty.
        let _ = remove_dir(&path);
        */
        
        // Create the directory.
        create_dir(&path)?;
        set_permissions(&path, Permissions::from_mode(0o775))?;
        Ok(Self {
            path: path.as_ref().to_owned(),
        })
    }

    /// Returns a reference to the path of this directory.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    /// Drop this TempDir by deleting its directory, if it is empty.
    ///
    /// If the directory is not empty, the directory is not deleted.
    fn drop(&mut self) {
        if let Err(e) = remove_dir(self.path()) {
            eprintln!("while deleting temporary directory {}: {}",
                      self.path().display(), e);
        }
    }
}


pub struct TempUnixDatagram {
    path: PathBuf,
    unix_datagram: UnixDatagram
}

impl TempUnixDatagram {
    /// Create a new TempUnixDatagram bound to the given path.
    ///
    /// This function may fail if the path cannot be created.
    pub fn bind<P: AsRef<Path>>(path: P) -> Result<Self> {
        let unix_datagram = UnixDatagram::bind(&path)?;
        set_permissions(&path, Permissions::from_mode(0o664))?;
        Ok(Self {
            path: path.as_ref().to_owned(),
            unix_datagram: unix_datagram,
        })
    }

    /// Returns a reference to the path of this file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempUnixDatagram {
    /// Drop this TempUnixDatagram by deleting it.
    fn drop(&mut self) {
        let _ = remove_file(self.path());
    }
}

impl Deref for TempUnixDatagram {
    type Target = UnixDatagram;
    /// Dereferences this TempUnixDatagram to its underlying UnixDatagram.
    fn deref(&self) -> &UnixDatagram {
        &self.unix_datagram
    }
}

impl DerefMut for TempUnixDatagram {
    /// Dereferences this TempUnixDatagram to its underlying File.
    fn deref_mut(&mut self) -> &mut UnixDatagram {
        &mut self.unix_datagram
    }
}

impl AsRef<UnixDatagram> for TempUnixDatagram {
    fn as_ref(&self) -> &UnixDatagram {
        self.deref()
    }
}

impl AsMut<UnixDatagram> for TempUnixDatagram {
    fn as_mut(&mut self) -> &mut UnixDatagram {
        self.deref_mut()
    }
}


pub struct TempUnixSeqpacketListener {
    path: PathBuf,
    unix_seqpacket_listener: UnixSeqpacketListener
}

impl TempUnixSeqpacketListener {
    /// Create a new TempUnixSeqpacketListener bound to the given path.
    ///
    /// This function may fail if the path cannot be created.
    pub fn bind<P: AsRef<Path>>(path: P) -> Result<Self> {
        let seqpacket_listener = UnixSeqpacketListener::bind(&path)?;
        set_permissions(&path, Permissions::from_mode(0o664))?;
        Ok(Self {
            path: path.as_ref().to_owned(),
            unix_seqpacket_listener: seqpacket_listener,
        })
    }

    /// Returns a reference to the path of this file.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempUnixSeqpacketListener {
    /// Drop this TempUnixSeqpacketListener by deleting it.
    fn drop(&mut self) {
        println!("Dropping temporary at {}", self.path().display());
        if let Err(e) = remove_file(self.path()) {
            eprintln!("Error deleting temporary seqpacket socket {}: {}",
                      self.path.display(), e);
        }
    }
}

impl Deref for TempUnixSeqpacketListener {
    type Target = UnixSeqpacketListener;
    /// Dereferences this TempUnixSeqpacketListener to its underlying UnixSeqpacketListener.
    fn deref(&self) -> &UnixSeqpacketListener {
        &self.unix_seqpacket_listener
    }
}

impl DerefMut for TempUnixSeqpacketListener {
    /// Dereferences this TempUnixSeqpacketListener to its underlying File.
    fn deref_mut(&mut self) -> &mut UnixSeqpacketListener {
        &mut self.unix_seqpacket_listener
    }
}

impl AsRef<UnixSeqpacketListener> for TempUnixSeqpacketListener {
    fn as_ref(&self) -> &UnixSeqpacketListener {
        self.deref()
    }
}

impl AsMut<UnixSeqpacketListener> for TempUnixSeqpacketListener {
    fn as_mut(&mut self) -> &mut UnixSeqpacketListener {
        self.deref_mut()
    }
}
pub trait FileExt: Sized {
    fn new_with_data<P: AsRef<Path>>(path: P, data: &[u8]) -> Result<Self>;
    fn write_to_start(&self, data: &[u8]) -> Result<()>;
}

impl FileExt for TempFile {
    fn new_with_data<P: AsRef<Path>>(p: P, data: &[u8]) -> Result<Self> {
        let file = TempFile::new(p)?;
        file.write_to_start(data)?;
        Ok(file)
    }

    fn write_to_start(&self, data: &[u8]) -> Result<()> {
        write(self.path(), data)
    }
}


