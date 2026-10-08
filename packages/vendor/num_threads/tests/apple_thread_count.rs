#![cfg(any(target_os = "macos", target_os = "ios"))]

extern crate num_threads;

#[test]
fn counts_a_live_worker_thread() {
    let initial = num_threads::num_threads().expect("Mach thread enumeration failed");
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
    });
    started_rx.recv().unwrap();
    let count = num_threads::num_threads();
    let single_threaded = num_threads::is_single_threaded();
    release_tx.send(()).unwrap();
    worker.join().unwrap();

    assert!(count.expect("Mach thread enumeration failed").get() > initial.get());
    assert_eq!(single_threaded, Some(false));
}
