// Existing adapter/lifetime contracts remain available for module contributions
// and isolated tests; the shipping catalog currently registers no adapters.
#![allow(dead_code)]

pub mod catalog;
pub mod codec;
pub(crate) mod configured_intent;
pub mod contract;
pub mod coordinator;
pub mod journal;
pub mod recovery;
pub mod runner;
pub mod writer;

#[cfg(all(test, windows, feature = "tuning-test-controls"))]
mod native_recovery_tests;
#[cfg(all(test, windows, feature = "tuning-test-controls"))]
mod publication_lab_tests;
