use super::*;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_ENCRYPTED, FILE_ATTRIBUTE_NORMAL};

#[test]
fn encrypted_directory_observation_is_refused() {
    assert!(
        matches!(
            observed_attributes(
                FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_ENCRYPTED.0,
                Kind::Directory
            ),
            Err(TuningNativeError::UnsafeStore(_))
        ),
        "encrypted observed directory must be refused"
    );
}

#[test]
fn encrypted_file_observation_is_refused() {
    assert!(
        matches!(
            observed_attributes(
                FILE_ATTRIBUTE_NORMAL.0 | FILE_ATTRIBUTE_ENCRYPTED.0,
                Kind::File
            ),
            Err(TuningNativeError::UnsafeStore(_))
        ),
        "encrypted observed file must be refused"
    );
}

#[test]
fn observed_attribute_type_and_reparse_contract_remains_strict() {
    assert!(observed_attributes(FILE_ATTRIBUTE_DIRECTORY.0, Kind::Directory).is_ok());
    assert!(observed_attributes(FILE_ATTRIBUTE_NORMAL.0, Kind::File).is_ok());
    for (attributes, kind) in [
        (FILE_ATTRIBUTE_NORMAL.0, Kind::Directory),
        (FILE_ATTRIBUTE_DIRECTORY.0, Kind::File),
        (FILE_ATTRIBUTE_REPARSE_POINT.0, Kind::File),
        (
            FILE_ATTRIBUTE_DIRECTORY.0 | FILE_ATTRIBUTE_REPARSE_POINT.0,
            Kind::Directory,
        ),
    ] {
        assert!(matches!(
            observed_attributes(attributes, kind),
            Err(TuningNativeError::UnsafeStore(_))
        ));
    }
}
