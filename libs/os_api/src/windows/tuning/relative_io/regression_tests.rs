use super::*;
use std::cell::RefCell;

#[test]
fn rename_completion_requires_exact_completed_status_without_transfer_count() {
    assert_eq!(set_completion(0, 0, 0), Ok(()));
    for (status, io, information) in [
        (0x103, 0, 0),
        (0, 0x103, 0),
        (1, 0, 0),
        (0, 1, 0),
        (0xc0000034_u32 as i32, 0, 0),
        (0, 0xc0000034_u32 as i32, 0),
        (0, 0, 1),
        (0, 0, usize::MAX),
    ] {
        assert!(set_completion(status, io, information).is_err());
    }
}

#[test]
fn rename_extent_contains_sdk_minimum_and_supplied_zero_wchar_tail() {
    for count in [1, 2, NAME_UNITS - 1, NAME_UNITS] {
        let name = RelativeName::new(&"x".repeat(count)).unwrap();
        let (storage, length) = rename_buffer(&name).unwrap();
        let name_bytes = count * size_of::<u16>();
        assert!(length >= size_of::<FILE_RENAME_INFORMATION>() + name_bytes);
        let start = offset_of!(FILE_RENAME_INFORMATION, FileName);
        assert!(length >= start + name_bytes + size_of::<u16>());
        let bytes = unsafe {
            std::slice::from_raw_parts(
                storage.as_ptr().cast::<u8>(),
                storage.len() * size_of::<u64>(),
            )
        };
        assert!(length <= bytes.len());
        assert_eq!(&bytes[start + name_bytes..start + name_bytes + 2], &[0, 0]);
        let info = unsafe { &*storage.as_ptr().cast::<FILE_RENAME_INFORMATION>() };
        assert!(!unsafe { info.Anonymous.ReplaceIfExists });
        assert!(info.RootDirectory.is_invalid());
        assert_eq!(info.FileNameLength as usize, name_bytes);
        assert_eq!(
            &bytes[start..start + name_bytes],
            "x".repeat(count)
                .encode_utf16()
                .flat_map(u16::to_le_bytes)
                .collect::<Vec<_>>()
        );
    }
    for units in [0, NAME_UNITS + 1, usize::MAX] {
        assert_eq!(
            rename_buffer_extent(units),
            Err(TuningNativeError::UnsafeStore("relative name"))
        );
    }
}

struct OutputDrop(Rc<RefCell<Vec<&'static str>>>);
impl Drop for OutputDrop {
    fn drop(&mut self) {
        self.0.borrow_mut().push("drop");
    }
}
#[test]
fn unexpected_pending_retains_output_allocation_without_read_or_drop() {
    let trace = Rc::new(RefCell::new(Vec::new()));
    let output = Box::new(OutputDrop(Rc::clone(&trace)));
    let result: Result<()> = inspect_completion_output(0x103, output, |_| {
        trace.borrow_mut().push("read");
        Ok(())
    });
    assert_eq!(
        result,
        Err(TuningNativeError::UnsafeStore(
            "relative pending completion"
        ))
    );
    assert!(
        trace.borrow().is_empty(),
        "pending output is neither read nor freed"
    );
}
#[test]
fn nonpending_completion_inspects_then_releases_owned_output() {
    for status in [0, 1, 0xc0000034_u32 as i32] {
        let trace = Rc::new(RefCell::new(Vec::new()));
        let output = Box::new(OutputDrop(Rc::clone(&trace)));
        let result: Result<()> = inspect_completion_output(status, output, |_| {
            trace.borrow_mut().push("read");
            Err(TuningNativeError::UnsafeStore("fixed synthetic completion"))
        });
        assert_eq!(
            result,
            Err(TuningNativeError::UnsafeStore("fixed synthetic completion"))
        );
        assert_eq!(*trace.borrow(), ["read", "drop"]);
    }
}
