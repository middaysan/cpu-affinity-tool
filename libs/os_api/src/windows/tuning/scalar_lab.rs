//! One fixed owned target; unresolved evidence is retained without Drop cleanup.
pub use super::store_io::native::TuningLabScalarIo;
use super::{
    security::{Descriptor, SecurityPolicy},
    store_io::{
        Disposition, Kind, OpenMode, Sharing,
        native::{Directory, Node},
    },
    test_support::TuningTemporaryDirectory,
};
use crate::tuning::TuningNativeError;
use std::rc::Rc;
const NAME: &str = "scalar-owned-lab.bin";
pub struct TuningLabScalarTarget {
    node: Option<Node>,
    directory: Option<Directory>,
    root: Option<TuningTemporaryDirectory>,
}
impl std::fmt::Debug for TuningLabScalarTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TuningLabScalarTarget(redacted)")
    }
}
impl Drop for TuningLabScalarTarget {
    fn drop(&mut self) {
        if let Some(root) = self.root.take() {
            root.retain();
        }
    }
}
impl TuningLabScalarTarget {
    pub fn new() -> Result<Self, TuningNativeError> {
        let root = TuningTemporaryDirectory::new()?;
        let mut owned = Self {
            node: None,
            directory: None,
            root: Some(root),
        };
        let sid = crate::windows::OS::tuning_current_token_identity()?
            .sid()
            .to_owned();
        let policy = Rc::new(SecurityPolicy {
            test_sid: Some(sid.clone()),
        });
        let descriptor = Rc::new(Descriptor::new(Some(&sid))?);
        let root = owned.root.as_ref().unwrap();
        let directory = Directory::bootstrap(root.path(), descriptor, policy, Some(true))?;
        let original = root.original_identity();
        if directory.id().volume != original.0 || directory.id().index != original.1 {
            return Err(TuningNativeError::UnsafeStore("owned scalar root changed"));
        }
        owned.directory = Some(directory);
        let (mut node, created) = owned.directory.as_ref().unwrap().open(
            &NAME.encode_utf16().collect::<Vec<_>>(),
            Kind::File,
            OpenMode::CreateNew,
            Sharing::Renameable,
            false,
            Some(true),
        )?;
        if created != Disposition::Created {
            return Err(TuningNativeError::UnsafeStore("owned scalar creation"));
        }
        node.write_synced(&7i32.to_le_bytes(), 4)?;
        owned.node = Some(node);
        Ok(owned)
    }
    pub fn io(&self) -> Result<TuningLabScalarIo, TuningNativeError> {
        self.node
            .as_ref()
            .ok_or(TuningNativeError::UnsafeStore("owned scalar closed"))?
            .scalar_io()
    }
    pub fn read(&mut self) -> Result<i32, TuningNativeError> {
        self.verify()?;
        let bytes = self
            .node
            .as_mut()
            .ok_or(TuningNativeError::UnsafeStore("owned scalar closed"))?
            .read(4)?;
        Ok(i32::from_le_bytes(bytes.try_into().map_err(|_| {
            TuningNativeError::UnsafeStore("owned scalar length")
        })?))
    }
    fn verify(&self) -> Result<(), TuningNativeError> {
        let error = || TuningNativeError::UnsafeStore("owned scalar namespace");
        let directory = self.directory.as_ref().ok_or_else(error)?;
        let node = self.node.as_ref().ok_or_else(error)?;
        directory.verify()?;
        node.verify()?;
        let original = self.root.as_ref().ok_or_else(error)?.original_identity();
        if directory.id().volume != original.0 || directory.id().index != original.1 {
            return Err(error());
        }
        let entries = directory.entries()?;
        if entries.len() != 1
            || entries[0].name != NAME.encode_utf16().collect::<Vec<_>>()
            || !directory.entry_matches(&entries[0], node)?
        {
            return Err(error());
        }
        Ok(())
    }
    pub fn restore(&mut self) -> Result<(), TuningNativeError> {
        if !matches!(self.read()?, 7 | 12) {
            return Err(TuningNativeError::UnsafeStore("owned scalar value"));
        }
        self.node
            .as_mut()
            .unwrap()
            .write_synced(&7i32.to_le_bytes(), 4)?;
        if self.read()? != 7 {
            return Err(TuningNativeError::UnsafeStore("owned scalar restore"));
        }
        Ok(())
    }
    pub fn cleanup(&mut self) -> Result<(), TuningNativeError> {
        if self.read()? != 7 {
            return Err(TuningNativeError::UnsafeStore(
                "owned scalar baseline required",
            ));
        }
        self.verify()?;
        self.node
            .as_ref()
            .unwrap()
            .clone_for_lab()?
            .delete_owned()?;
        drop(self.node.take());
        if !self.directory.as_ref().unwrap().entries()?.is_empty() {
            return Err(TuningNativeError::UnsafeStore("owned scalar namespace"));
        }
        drop(self.directory.take());
        self.root.as_mut().unwrap().cleanup_empty()?;
        drop(self.root.take());
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn remove_fixture(mut target: TuningLabScalarTarget) {
        target.node.take().unwrap().delete_owned().unwrap();
        drop(target.directory.take());
        target.root.take().unwrap().cleanup().unwrap();
    }
    #[test]
    fn owned_scalar_io_reads_writes_actual_retained_four_bytes() {
        let mut target = TuningLabScalarTarget::new().unwrap();
        assert_eq!(target.read().unwrap(), 7);
        let io = target.io().unwrap();
        let read = io.read();
        let write = io.write(12);
        let physical = target.read().unwrap();
        drop(io);
        remove_fixture(target);
        assert_eq!(read, Ok(7));
        assert_eq!(write, Ok(()));
        assert_eq!(physical, 12);
    }
    #[test]
    fn owned_scalar_independent_restore_and_exact_cleanup_are_explicit() {
        let mut target = TuningLabScalarTarget::new().unwrap();
        target
            .node
            .as_mut()
            .unwrap()
            .write_synced(&12i32.to_le_bytes(), 4)
            .unwrap();
        let restored = target.restore();
        let physical = target.read().unwrap();
        let cleaned = target.cleanup();
        if cleaned.is_err() {
            remove_fixture(target);
        }
        assert_eq!(restored, Ok(()));
        assert_eq!(physical, 7);
        assert_eq!(cleaned, Ok(()));
    }
    #[test]
    fn scalar_capability_refuses_foreign_thread_without_native_write() {
        let mut target = TuningLabScalarTarget::new().unwrap();
        let io = target.io().unwrap();
        let (read, write) = std::thread::spawn(move || (io.read(), io.write(12)))
            .join()
            .unwrap();
        let physical = target.read().unwrap();
        target.cleanup().unwrap();
        assert_eq!(
            read,
            Err(TuningNativeError::UnsafeStore("owned scalar worker"))
        );
        assert_eq!(
            write,
            Err(TuningNativeError::UnsafeStore("owned scalar worker"))
        );
        assert_eq!(physical, 7);
    }
    #[test]
    fn scalar_cleanup_unknown_entry_refuses_before_owned_target_deletion() {
        let mut target = TuningLabScalarTarget::new().unwrap();
        let root = target.root.as_ref().unwrap().path().to_owned();
        std::fs::write(root.join("fixture-unknown"), b"retained evidence").unwrap();
        let result = target.cleanup();
        assert_eq!(std::fs::read(root.join(NAME)).unwrap(), 7i32.to_le_bytes());
        assert_eq!(
            std::fs::read(root.join("fixture-unknown")).unwrap(),
            b"retained evidence"
        );
        std::fs::remove_file(root.join("fixture-unknown")).unwrap();
        target.cleanup().unwrap();
        assert!(result.is_err());
    }
}
