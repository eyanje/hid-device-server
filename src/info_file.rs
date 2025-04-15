use std::ops::Deref;
use std::path::Path;

use crate::fs::{FileExt, TempFile};
use crate::registration::RegistrationInfo;

pub fn create_info_files<P: Deref<Target = Path>, Id>(
    base_path: P,
    current_registration_info: &RegistrationInfo<Id>,
) -> Result<Vec<TempFile>, std::io::Error> {
    let mut static_info_files = Vec::new();

    Ok(static_info_files)
}
