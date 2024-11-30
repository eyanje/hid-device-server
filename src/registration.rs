use bluer::rfcomm::ProfileHandle;
use btmgmt::command::SetDeviceClass;
use hid_device_class::{MajorDeviceClass, Peripheral};
use hid_device_configuration::{Configuration, PartialConfiguration};
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
    sdp_record: String,
    configuration: Configuration,
}

impl<Id> RegistrationInfo<Id> {
    pub fn registrant(&self) -> &Id {
        &self.registrant
    }

    pub fn sdp_record(&self) -> &str {
        &self.sdp_record
    }

    pub fn configuration(&self) -> &Configuration {
        &self.configuration
    }
}

#[derive(Debug)]
pub struct Registration<Id> {
    info: RegistrationInfo<Id>,
    _profile_handle: ProfileHandle,
}

impl<Id> Registration<Id> {
    pub async fn register(
        registrant: Id,
        sdp_record: String,
    ) -> Result<Self, Error> {

        // Parse SDP record
        let configuration: Configuration = PartialConfiguration::from_sdp_xml(sdp_record.as_bytes())?
            .try_into()?;
        println!("New configuration: {:?}", configuration);

        // Set the device class
        let set_device_class = SetDeviceClass::new(
            (Peripheral::major_device_class() >> 8).try_into().unwrap(),
            configuration.hid.device_subclass);
        let client = btmgmt::Client::open()?;
        client.call(Some(0), set_device_class).await?;

        // Register a BlueZ profile
        let session = bluer::Session::new().await.unwrap();
        let profile_handle_opt = session.register_profile(bluer::rfcomm::Profile {
            service_record: Some(sdp_record.clone()),
            ..bluer::rfcomm::Profile::default()
        }).await;

        // Store the profile handle in state
        let profile_handle = match profile_handle_opt {
            Ok(profile_handle) => profile_handle,
            Err(e) => {
                return Err(Error::Bluer(e));
            }
        };

        // Return a struct holding the registration.
        Ok(Self {
            info: RegistrationInfo {
                registrant,
                sdp_record,
                configuration,
            },
            _profile_handle: profile_handle,
        })
    }

    pub fn info(&self) -> &RegistrationInfo<Id> {
        &self.info
    }
}


