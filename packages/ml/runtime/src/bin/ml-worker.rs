use flow_like_ml_runtime::{
    engines::{register_burn_engine, register_efficient_ad_engine, register_native_engines},
    worker::TrainingWorker,
    *,
};

fn main() {
    match run() {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

/// The supervisor chooses the ledger and job explicitly. Model recipes never select a
/// shell command or executable. Cancellation is a durable request in the same ledger.
fn run() -> Result<serde_json::Value> {
    let mut args = std::env::args().skip(1);
    let mut repository = None;
    let mut job = None;
    let mut cancel = None;
    let mut list = false;
    let mut resume = false;
    let mut limits = WorkerLimits::default();
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--repository" => {
                repository = Some(
                    args.next()
                        .ok_or_else(|| Error::Invalid("--repository requires a path".into()))?,
                )
            }
            "--job" => {
                job = Some(
                    args.next()
                        .ok_or_else(|| Error::Invalid("--job requires an ID".into()))?,
                )
            }
            "--cancel" => {
                cancel = Some(
                    args.next()
                        .ok_or_else(|| Error::Invalid("--cancel requires an ID".into()))?,
                )
            }
            "--list-pending" => list = true,
            "--resume" => resume = true,
            "--memory-bytes" => limits.memory_budget_bytes = number(args.next(), "--memory-bytes")?,
            "--timeout-ms" => limits.maximum_duration_ms = number(args.next(), "--timeout-ms")?,
            _ => {
                return Err(Error::Invalid(format!(
                    "unknown argument {argument}; use --repository PATH with --job ID, --cancel ID, or --list-pending"
                )));
            }
        }
    }
    if usize::from(job.is_some()) + usize::from(cancel.is_some()) + usize::from(list) != 1
        || (resume && job.is_none())
    {
        return Err(Error::Invalid(
            "choose exactly one of --job, --cancel or --list-pending; --resume requires --job"
                .into(),
        ));
    }
    let repository = TrainingRepository::open(
        repository.ok_or_else(|| Error::Invalid("--repository is required".into()))?,
    )?;
    if let Some(job) = cancel {
        return Ok(serde_json::to_value(
            repository.request_cancel(&job, now_ms())?,
        )?);
    }
    if list {
        return Ok(serde_json::to_value(repository.pending_jobs()?)?);
    }
    let job = job.unwrap();
    if resume {
        repository.resume_job(&job, now_ms())?;
    }
    let mut worker = TrainingWorker::new(repository, limits)?;
    register_native_engines(&mut worker)?;
    register_burn_engine(&mut worker)?;
    register_efficient_ad_engine(&mut worker)?;
    Ok(serde_json::to_value(worker.run(&job)?)?)
}
fn number(value: Option<String>, name: &str) -> Result<u64> {
    value
        .and_then(|s| s.parse().ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| Error::Invalid(format!("{name} requires a positive integer")))
}
