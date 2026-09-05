#[tokio::main(worker_threads = 2)]
async fn main() {
    if execution_manager::kubernetes::slot::main().await.is_err() {
        eprintln!("Execution slot failed; inspect manager and Pod status");
        std::process::exit(1);
    }
}
