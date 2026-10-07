use super::*;
use std::{sync::mpsc, thread, time::Duration};

#[test]
fn nonterminal_request_is_not_dropped_when_same_worker_is_retained() {
    struct Request(mpsc::Sender<()>);
    impl Drop for Request {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let (drop_tx, drop_rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        retain_nonterminal(Box::new(Request(drop_tx)), move || {
            entered_tx.send(()).unwrap();
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(
        matches!(
            drop_rx.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ),
        "same nonterminal worker owns its complete request"
    );
    assert!(
        !worker.is_finished(),
        "nonterminal worker cannot report completion"
    );
    // This pure worker owns no native authority. Dropping its controller is a
    // deliberate characterization of indefinite retention until process exit.
    drop(worker);
}
