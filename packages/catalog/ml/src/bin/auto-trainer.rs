use flow_like_catalog_ml::inspection::auto_training::{
    AutoTrainingResult, controller, resume_saved_experiment,
};
use flow_like_ml_runtime::{TrainingRepository, now_ms};
use flow_like_types::{Result, Value, anyhow, json};

fn main() {
    match run() {
        Ok(result) => println!("{result}"),
        Err(error) => {
            eprintln!("{error:#}");
            std::process::exit(1);
        }
    }
}

fn run() -> Result<Value> {
    let mut args = std::env::args().skip(1);
    let mut repository = None;
    let mut experiment = None;
    let mut mode = "run";
    let mut resume = false;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--repository" => {
                repository = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--repository requires a path"))?,
                )
            }
            "--experiment" => {
                experiment = Some(
                    args.next()
                        .ok_or_else(|| anyhow!("--experiment requires an ID"))?,
                )
            }
            "--result" | "--cancel" | "--step" if mode == "run" => {
                mode = match argument.as_str() {
                    "--result" => "result",
                    "--cancel" => "cancel",
                    _ => "step",
                }
            }
            "--resume" => resume = true,
            _ => {
                return Err(anyhow!(
                    "Use --repository PATH --experiment ID with one optional --result, --cancel or --step; --resume resumes saved work"
                ));
            }
        }
    }
    if resume && matches!(mode, "result" | "cancel") {
        return Err(anyhow!("--resume requires run or --step"));
    }
    let repo =
        TrainingRepository::open(repository.ok_or_else(|| anyhow!("--repository is required"))?)?;
    let id = experiment.ok_or_else(|| anyhow!("--experiment is required"))?;
    if resume {
        resume_saved_experiment(&repo, &id)?;
    }
    match mode {
        "result" => {}
        "cancel" => {
            repo.cancel_experiment(&id, now_ms())?;
        }
        _ => {
            let experiment = repo.get_experiment(&id)?;
            let engine = controller(repo.clone(), &experiment.request.budget)?;
            if mode == "step" {
                engine.run_next(&id)?;
            } else {
                engine.run(&id)?;
            }
        }
    }
    Ok(json::to_value(AutoTrainingResult::from(
        repo.experiment_result(&id)?,
    ))?)
}
