use std::fmt::{self, Display, Formatter};
use std::string::FromUtf8Error;


#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Bluer(bluer::Error),
    Btmgmt(btmgmt::client::Error),
    Configuration(hid_device_configuration::Error),
    FromUtf8(FromUtf8Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> Result<(), fmt::Error> {
        match self {
            Error::Io(e) => write!(f, "{}", e),
            Error::Bluer(e) => write!(f, "BlueR: {}", e),
            Error::Btmgmt(e) => write!(f, "btmgmt error: {}", e),
            Error::Configuration(e) => write!(f, "configuration error: {}", e),
            Error::FromUtf8(e) => write!(f, "converting to utf8: {}", e),
        }
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
impl From<bluer::Error> for Error {
    fn from(e: bluer::Error) -> Self {
        Self::Bluer(e)
    }
}
impl From<btmgmt::client::Error> for Error {
    fn from(e: btmgmt::client::Error) -> Self {
        Self::Btmgmt(e)
    }
}
impl From<hid_device_configuration::Error> for Error {
    fn from(e: hid_device_configuration::Error) -> Self {
        Self::Configuration(e)
    }
}
impl From<FromUtf8Error> for Error {
    fn from(e: FromUtf8Error) -> Self {
        Self::FromUtf8(e)
    }
}

#[derive(Clone, Debug)]
pub struct RegistrationInfo<Id> {
    registrant: Id,
}

impl<Id> RegistrationInfo<Id> {
    pub fn registrant(&self) -> &Id {
        &self.registrant
    }
}

#[derive(Debug)]
pub struct Registration<Id> {
    info: RegistrationInfo<Id>,
}

impl<Id> Registration<Id> {
    pub async fn register(
        registrant: Id,
    ) -> Result<Self, Error> {
        // Return a struct holding the registration.
        Ok(Self {
            info: RegistrationInfo {
                registrant,
            },
        })
    }

    pub fn info(&self) -> &RegistrationInfo<Id> {
        &self.info
    }
}


