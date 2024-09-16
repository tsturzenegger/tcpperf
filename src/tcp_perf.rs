use clap::{Parser, ValueEnum};
use pbr::{ProgressBar, Units};
use std::{error::Error, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const BUF_SIZE: usize = 4_096; // Pagesize (getconf PAGESIZE)
const PROGRESS_BAR_REFRESH_RATE_SEC: u64 = 1; //second
const MB_IN_BYTE: u64 = 1024 * 1024;
const DEFAULT_SAMPLE_SIZE: u64 = 100;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Mode {
    Server,
    Client,
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(value_enum)]
    mode: Mode,

    /// ip_addr:port e.g. 127.0.0.1:8080
    #[arg(default_value = "localhost:8080")]
    addr: String,

    /// Sample size in MByte
    #[arg(short, long, default_value_t = DEFAULT_SAMPLE_SIZE)]
    sample_size: u64,
}

#[derive(Clone, Copy)]
pub struct TcpPerf {
    mode: Mode,
    sample_size: u64,
}

impl TcpPerf {
    pub async fn start() -> Result<(), Box<dyn Error>> {
        let cli = Cli::parse();

        match cli.mode {
            Mode::Server => {
                let listener = TcpListener::bind(cli.addr).await?;
                let tester = TcpPerf {
                    mode: Mode::Server,
                    sample_size: 0, // use 0 because sample will be assigned later
                };
                loop {
                    let (stream, _) = listener.accept().await?;
                    tokio::spawn(async move {
                        if let Ok(addr) = stream.peer_addr() {
                            println!("new connection from {}", addr);
                            match tester.handle_client(stream).await {
                                Ok(n) => println!(
                                    "succesfully tested {} (sample size: {} MB)",
                                    addr,
                                    n / MB_IN_BYTE
                                ),
                                Err(e) => println!("Error handling client ({:?}): {:?}", addr, e),
                            }
                        }
                    });
                }
            }
            Mode::Client => {
                let tester = TcpPerf {
                    mode: Mode::Client,
                    sample_size: cli
                        .sample_size
                        .checked_mul(MB_IN_BYTE)
                        .unwrap_or(DEFAULT_SAMPLE_SIZE),
                };
                tester
                    .handle_server(TcpStream::connect(cli.addr).await?)
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_client(mut self, mut stream: TcpStream) -> Result<u64, Box<dyn Error>> {
        self.sample_size = self.download(&mut stream).await?;
        self.upload(&mut stream).await?;
        Ok(self.sample_size)
    }

    async fn handle_server(self, mut stream: TcpStream) -> Result<(), Box<dyn Error>> {
        self.upload(&mut stream).await?;
        self.download(&mut stream).await?;
        Ok(())
    }

    async fn upload(self, stream: &mut TcpStream) -> Result<(), Box<dyn Error>> {
        let buf: [u8; BUF_SIZE] = core::array::from_fn(|i| ((i + 1) % u8::MAX as usize) as u8); // Fill buf with "nonsense" values
        let mut byte_counter: u64 = 0;
        match self.mode {
            Mode::Client => {
                let mut pb = ProgressBar::new(self.sample_size);
                pb.set_units(Units::Bytes);
                pb.set_max_refresh_rate(Some(Duration::from_secs(PROGRESS_BAR_REFRESH_RATE_SEC)));
                pb.format("╢▌▌░╟");
                stream
                    .write_all(&u64::to_le_bytes(self.sample_size))
                    .await?;
                while byte_counter < self.sample_size {
                    stream.write_all(&buf).await?;
                    byte_counter += buf.len() as u64;
                    pb.set(byte_counter);
                }
                pb.finish_println("finished uploading...\n");
            }
            Mode::Server => {
                stream
                    .write_all(&u64::to_le_bytes(self.sample_size))
                    .await?;
                while byte_counter < self.sample_size {
                    stream.write_all(&buf).await?;
                    byte_counter += buf.len() as u64;
                }
            }
        }
        Ok(())
    }

    async fn download(self, stream: &mut TcpStream) -> Result<u64, Box<dyn Error>> {
        let mut buf = vec![0; BUF_SIZE]; // Vec is intentional see: https://tokio.rs/tokio/tutorial/io
        let mut byte_counter: u64 = 0;
        let mut sample_size_buf: [u8; 8] = [0; 8];
        stream.read_exact(&mut sample_size_buf).await?;
        let sample_size = u64::from_le_bytes(sample_size_buf);
        match self.mode {
            Mode::Client => {
                let mut pb = ProgressBar::new(sample_size);
                pb.set_units(Units::Bytes);
                pb.set_max_refresh_rate(Some(Duration::from_secs(PROGRESS_BAR_REFRESH_RATE_SEC)));
                pb.format("╢▌▌░╟");
                while byte_counter < sample_size {
                    match stream.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            byte_counter += n as u64;
                            pb.set(byte_counter);
                        }
                        Err(e) => return Err(Box::new(e)),
                    }
                }
                pb.finish_println("finished downloading...\n");
            }
            Mode::Server => {
                while byte_counter < sample_size {
                    match stream.read(&mut buf).await {
                        Ok(0) => break,
                        Ok(n) => {
                            byte_counter += n as u64;
                        }
                        Err(e) => return Err(Box::new(e)),
                    }
                }
            }
        }
        Ok(byte_counter)
    }
}
