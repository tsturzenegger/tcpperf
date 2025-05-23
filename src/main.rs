use clap::{Parser, ValueEnum};
use tokio::net::{TcpListener, TcpStream};
use tcpperf::{Mode, TcpPerf, TcpPerfError, MB_IN_BYTE, DEFAULT_SAMPLE_SIZE, BUF_SIZE as LIB_BUF_SIZE}; // Import current BUF_SIZE as LIB_BUF_SIZE

// --- CLI Argument Parsing Structs ---

/// Default buffer size in Kilobytes for CLI.
const DEFAULT_BUF_SIZE_KB: usize = LIB_BUF_SIZE / 1024; // Use current lib BUF_SIZE as default

/// Defines the command-line arguments accepted by the `tcpperf` utility.
/// Uses `clap` for parsing and validation.
#[derive(Parser, Debug)]
#[command(version, about, long_about = "A simple TCP performance testing tool.")]
struct Cli {
    /// Mode of operation: Server or Client.
    #[arg(value_enum)]
    mode: CLIMode,

    /// Network address to bind to (for Server) or connect to (for Client).
    /// Format: "ip_addr:port" (e.g., "127.0.0.1:8080" or "localhost:8080").
    #[arg(default_value = "localhost:8080")]
    addr: String,

    /// Sample size in Megabytes (MB) for data transfer.
    /// This value is used by the client to determine how much data to send,
    /// and by the server (after learning from client) to send back.
    #[arg(short, long, default_value_t = DEFAULT_SAMPLE_SIZE)]
    sample_size: u64,

    /// Buffer size in Kilobytes (KB) for read/write operations.
    #[arg(long, default_value_t = DEFAULT_BUF_SIZE_KB)]
    buf_size_kb: usize,
}

/// Enum defining the operational modes for CLI argument parsing.
/// This is distinct from the internal `tcpperf::Mode` to leverage `clap::ValueEnum`.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum CLIMode {
    Server,
    Client,
}

// --- Main Application Logic ---

/// Main entry point for the `tcpperf` application.
/// Parses command-line arguments, initializes `TcpPerf` in the specified mode,
/// and starts the server or client operation.
#[tokio::main]
async fn main() -> Result<(), ()> { // Return empty tuple on error to prevent default Display trait output from main
    let cli = Cli::parse();

    // Convert CLI mode to internal application mode.
    let app_mode = match cli.mode {
        CLIMode::Server => Mode::Server,
        CLIMode::Client => Mode::Client,
    };

    // Execute corresponding logic based on the application mode.
    let result = match app_mode {
        Mode::Server => run_server(cli).await,
        Mode::Client => run_client(cli).await,
    };

    if let Err(e) = result {
        eprintln!("Application error: {}", e); // Print custom error using its Display impl
        return Err(()); // Indicate failure
    }
    Ok(())
}

/// Runs the TcpPerf logic in Server mode.
/// Binds to the specified address and listens for incoming client connections.
/// Each connection is handled in a separate asynchronous task.
async fn run_server(cli: Cli) -> Result<(), TcpPerfError> {
    let listener = TcpListener::bind(&cli.addr).await?; // `?` implicitly uses `From<std::io::Error>` for TcpPerfError
    println!("Server listening on {}...", cli.addr);
    let buf_size_bytes = cli.buf_size_kb * 1024; // Calculate once
    loop {
        let (stream, client_addr) = listener.accept().await?;
        println!("New connection from {}", client_addr);
        tokio::spawn(async move {
            // Server's sample_size is determined by client during the download phase.
            let mut tester = TcpPerf::new_server(buf_size_bytes); // Pass buf_size_bytes
            if let Err(e) = handle_client_connection(&mut tester, stream).await {
                eprintln!("Error handling client {}: {}", client_addr, e);
            }
        });
    }
    // Note: Server loop runs indefinitely unless an error occurs with the listener itself.
}

/// Runs the TcpPerf logic in Client mode.
/// Connects to the specified server address and performs upload and download operations.
async fn run_client(cli: Cli) -> Result<(), TcpPerfError> {
    println!("Client connecting to {}...", cli.addr);
    let stream = TcpStream::connect(&cli.addr).await?;
    
    // Calculate sample size in bytes for the TcpPerf instance.
    let sample_size_bytes = cli.sample_size.checked_mul(MB_IN_BYTE)
        .unwrap_or_else(|| DEFAULT_SAMPLE_SIZE.checked_mul(MB_IN_BYTE).unwrap()); // Fallback to default if multiplication overflows.
    let buf_size_bytes = cli.buf_size_kb * 1024;
    
    let mut tester = TcpPerf::new(Mode::Client, sample_size_bytes, buf_size_bytes); // Pass buf_size_bytes
    handle_server_interaction(&mut tester, stream).await?; // Propagate errors to main for uniform handling
    Ok(())
}

// --- Connection Handling Logic ---

/// Handles a single client connection on the server side.
///
/// This function orchestrates the download (from client) and then upload (to client) sequence.
/// The server learns the `sample_size` from the client during the download phase.
async fn handle_client_connection(tester: &mut TcpPerf, mut stream: TcpStream) -> Result<(), TcpPerfError> {
    // Server first downloads data from the client.
    // The `download` method updates `tester.sample_size` with the size received from the client.
    let downloaded_payload_size = tester.download(&mut stream).await?;
    println!(
        "Server downloaded {} MB from client ({} bytes).", 
        downloaded_payload_size / MB_IN_BYTE, 
        downloaded_payload_size
    );
    
    // Then, server uploads the same amount of data back to the client.
    // `tester.sample_size` (in bytes) is now set to what the client initially sent.
    println!(
        "Server uploading {} MB to client ({} bytes).", 
        tester.sample_size / MB_IN_BYTE,
        tester.sample_size
    );
    tester.upload(&mut stream).await?; // Pass as mutable reference
    Ok(())
}

/// Handles the client's interaction with the server.
///
/// This function orchestrates the upload (to server) and then download (from server) sequence.
async fn handle_server_interaction(tester: &mut TcpPerf, mut stream: TcpStream) -> Result<(), TcpPerfError> {
    // Client first uploads data based on its `sample_size`.
    println!(
        "Client uploading {} MB to server ({} bytes).", 
        tester.sample_size / MB_IN_BYTE,
        tester.sample_size
    );
    tester.upload(&mut stream).await?; // Pass as mutable reference
    
    // Then, client downloads data from the server.
    // The `download` method will read the sample size header from the server.
    let downloaded_payload_size = tester.download(&mut stream).await?;
    println!(
        "Client downloaded {} MB from server ({} bytes).", 
        downloaded_payload_size / MB_IN_BYTE,
        downloaded_payload_size
    );
    Ok(())
}
