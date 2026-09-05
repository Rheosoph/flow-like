fn main() {
    if execution_manager::gateway::main().is_err() {
        // Policy and destination errors may contain credentials. Keep process
        // logs independent of tenant input and let the supervisor report failure.
        eprintln!("Execution gateway stopped after an initialization or supervision error");
        std::process::exit(1);
    }
}
