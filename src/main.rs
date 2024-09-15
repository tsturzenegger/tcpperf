use std::error::Error;

mod tcp_perf;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tcp_perf::TcpPerf::start().await?;
    Ok(())
}
