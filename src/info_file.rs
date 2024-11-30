use std::ops::Deref;
use std::path::Path;

use crate::fs::{FileExt, TempFile};
use crate::registration::RegistrationInfo;

use hid_device_configuration::hid::{ClassDescriptor, descriptor_type};

pub fn create_info_files<P: Deref<Target = Path>, Id>(
    base_path: P,
    current_registration_info: &RegistrationInfo<Id>,
) -> Result<Vec<TempFile>, std::io::Error> {
    let mut static_info_files = Vec::new();

    // SDP record
    {
        let path = base_path.join("sdp.xml");
        let data = current_registration_info.sdp_record().as_bytes();
        let file = TempFile::new_with_data(path, data)?;
        static_info_files.push(file);
    }

    // Service name
    if let Some(service_name) = &current_registration_info.configuration().service_name {
        let path = base_path.join("service_name");
        let data = service_name.as_bytes();
        let file = TempFile::new_with_data(path, data)?;
        static_info_files.push(file);
    }

    // Service Description
    if let Some(service_description) = &current_registration_info.configuration().service_description {
        let path = base_path.join("service_description");
        let data = service_description.as_bytes();
        let file = TempFile::new_with_data(path, data)?;
        static_info_files.push(file);
    }

    // Provider name
    if let Some(provider_name) = &current_registration_info.configuration().provider_name {
        let path = base_path.join("provider_name");
        let data = provider_name.as_bytes();
        let file = TempFile::new_with_data(path, data)?;
        static_info_files.push(file);
    }

    // Device subclass
    {
        let path = base_path.join("device_subclass");
        let data_string = format!("{}", current_registration_info.configuration().hid.device_subclass);
        let file = TempFile::new_with_data(path, data_string.as_bytes())?;
        static_info_files.push(file);
    }

    // Class descriptors
    for class_descriptor in &current_registration_info.configuration().hid.class_descriptors {
        match class_descriptor {
            ClassDescriptor(descriptor_type::REPORT, data) => {
                let path = base_path.join("report_descriptor");
                let file = TempFile::new_with_data(path, &data)?;
                static_info_files.push(file);
            },
            ClassDescriptor(descriptor_type::PHYSICAL, data) => {
                let path = base_path.join("physical_descriptor");
                let file = TempFile::new_with_data(path, &data)?;
                static_info_files.push(file);
            },
            ClassDescriptor(_, _) => (),
        }
    }

    Ok(static_info_files)
}
