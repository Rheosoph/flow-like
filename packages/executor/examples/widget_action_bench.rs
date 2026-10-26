#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    flow_like_executor::benchmark::serve().await?;
    Ok(())
}
