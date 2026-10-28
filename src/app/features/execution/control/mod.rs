pub(crate) mod policy;
mod runtime;
#[cfg(all(target_os = "windows", feature = "windows"))]
pub(crate) use runtime::RuleInfo;
pub(crate) use runtime::{Authority, SourceRequest, View, WriteRequest};

#[cfg(test)]
pub(crate) mod tests;
