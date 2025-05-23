use pbr::{ProgressBar, Units};
use std::{fmt, time::Duration}; // Added fmt for Display
use tokio::io::{AsyncReadExt, AsyncWriteExt};
// tokio::net::TcpStream was unused as methods are generic over S.

// Custom Error Type
#[derive(Debug)]
pub enum TcpPerfError {
    /// Represents an I/O error that occurred during network operations.
    IoError(std::io::Error),
    /// Represents an error that occurred when trying to parse a network address.
    AddressParseError(String),
    /// Indicates that the connection was closed unexpectedly before all data was transferred.
    ConnectionClosed,
    // Add other specific error variants as the application grows.
}

impl fmt::Display for TcpPerfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TcpPerfError::IoError(e) => write!(f, "I/O error: {}", e),
            TcpPerfError::AddressParseError(addr) => write!(f, "Failed to parse address: {}", addr),
            TcpPerfError::ConnectionClosed => write!(f, "Connection closed unexpectedly"),
        }
    }
}

impl std::error::Error for TcpPerfError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            TcpPerfError::IoError(e) => Some(e),
            _ => None,
        }
    }
}

// Helper for converting std::io::Error to TcpPerfError.
// This allows for easy conversion using `?` operator on I/O results.
impl From<std::io::Error> for TcpPerfError {
    fn from(err: std::io::Error) -> TcpPerfError {
        TcpPerfError::IoError(err)
    }
}

// --- Constants ---

/// Buffer size for reading and writing data, typically page size.
pub const BUF_SIZE: usize = 4_096;
/// Refresh rate for the progress bar in seconds.
pub const PROGRESS_BAR_REFRESH_RATE_SEC: u64 = 1;
/// Conversion factor from Megabytes to bytes. Used in CLI and reporting.
pub const MB_IN_BYTE: u64 = 1024 * 1024;
/// Default sample size in Megabytes if not specified by the user.
pub const DEFAULT_SAMPLE_SIZE: u64 = 100;

// --- Enums and Structs ---

/// Defines the operational mode of TcpPerf (Server or Client).
/// This enum is used internally and distinct from the CLI-specific mode enum.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mode {
    Server,
    Client,
}

/// Main struct for TCP performance testing.
/// Holds the operational mode, sample size, and buffer size for data transfer.
#[derive(Clone, Copy, Debug)]
pub struct TcpPerf {
    /// Current operational mode (Server or Client).
    pub mode: Mode,
    /// Sample size in bytes for upload/download operations.
    /// For Server mode during download, this is updated with the size received from the client.
    pub sample_size: u64,
    /// Buffer size in bytes for read/write operations.
    pub buf_size: usize,
}

impl TcpPerf {
    /// Creates a new `TcpPerf` instance.
    ///
    /// # Arguments
    /// * `mode` - The operational mode (`Server` or `Client`).
    /// * `sample_size_bytes` - The total number of bytes for the data transfer.
    /// * `buf_size_bytes` - The buffer size in bytes for network operations.
    pub fn new(mode: Mode, sample_size_bytes: u64, buf_size_bytes: usize) -> Self {
        TcpPerf {
            mode,
            sample_size: sample_size_bytes,
            buf_size: buf_size_bytes,
        }
    }
    
    /// Creates a new `TcpPerf` instance specifically for server mode,
    /// where the sample size is initially unknown and buffer size is specified.
    ///
    /// # Arguments
    /// * `buf_size_bytes` - The buffer size in bytes for network operations.
    pub fn new_server(buf_size_bytes: usize) -> Self {
        TcpPerf {
            mode: Mode::Server,
            sample_size: 0, // Updated by the `download` method upon receiving client's sample size.
            buf_size: buf_size_bytes,
        }
    }

    /// Creates and configures a progress bar for displaying data transfer progress.
    /// This is a private helper method used by `upload` and `download`.
    fn _create_progress_bar(total_bytes: u64) -> ProgressBar<std::io::Stderr> { // Specify type for ProgressBar
        let mut pb = ProgressBar::on(std::io::stderr(), total_bytes); // Use stderr
        pb.set_units(Units::Bytes);
        pb.set_max_refresh_rate(Some(Duration::from_secs(PROGRESS_BAR_REFRESH_RATE_SEC)));
        pb.format("╢▌▌░╟");
        pb
    }

    // Upload method - takes mutable self if it needs to change its state, otherwise self or &self
    // The original took `self` (owned), which is fine if TcpPerf is cheap to clone/copy.
    // For methods that don't modify TcpPerf's state but use its fields, `&self` is common.
    // If it modifies `self.sample_size` (like download does), it needs `&mut self`.
    // The original `upload` took `self`. Let's keep it `self` for now, assuming it's Copy.
    // --- Public Methods ---

    /// Performs the upload operation.
    ///
    /// Sends a predefined data pattern over the stream. The client mode shows a progress bar.
    /// The first 8 bytes sent define the total payload size that will follow.
    ///
    /// # Type Parameters
    /// * `S` - A stream type that implements `AsyncRead`, `AsyncWrite`, and `Unpin`.
    ///
    /// # Arguments
    /// * `self` - Consumes `TcpPerf` as it represents a complete operation.
    /// * `stream` - The mutable reference to the network stream.
    ///
    /// # Errors
    /// Returns `TcpPerfError` if any I/O error occurs during the upload.
    pub async fn upload<S>(self, stream: &mut S) -> Result<(), TcpPerfError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + ?Sized,
    {
        // Define a repeatable byte pattern for the buffer.
        let mut buf = vec![0u8; self.buf_size];
        for i in 0..self.buf_size {
            buf[i] = ((i + 1) % u8::MAX as usize) as u8;
        }
        let mut byte_counter: u64 = 0;
        
        // Phase 1: Send the total sample size (payload size) as an 8-byte little-endian integer.
        stream
            .write_all(&self.sample_size.to_le_bytes())
            .await?;

        // Handle zero sample size: if no data is to be sent, complete early.
        if self.sample_size == 0 {
            if self.mode == Mode::Client {
                let mut pb = Self::_create_progress_bar(0);
                pb.finish_println("finished uploading (0 bytes)...\n");
            }
            return Ok(());
        }

        // Phase 2: Send the actual payload data.
        match self.mode {
            Mode::Client => {
                let mut pb = Self::_create_progress_bar(self.sample_size);
                while byte_counter < self.sample_size {
                    let bytes_to_write = std::cmp::min(self.buf_size as u64, self.sample_size - byte_counter) as usize;
                    stream.write_all(&buf[..bytes_to_write]).await?;
                    byte_counter += bytes_to_write as u64;
                    pb.set(byte_counter);
                }
                pb.finish_println("finished uploading...\n");
            }
            Mode::Server => {
                while byte_counter < self.sample_size {
                     let bytes_to_write = std::cmp::min(self.buf_size as u64, self.sample_size - byte_counter) as usize;
                    stream.write_all(&buf[..bytes_to_write]).await?;
                    byte_counter += bytes_to_write as u64;
                }
            }
        }
        Ok(())
    }

    /// Performs the download operation.
    ///
    /// Reads data from the stream. The client mode shows a progress bar.
    /// The first 8 bytes received define the total payload size to expect.
    /// For Server mode, this method updates `self.sample_size` with the size received from the client,
    /// which is then used for the subsequent server upload.
    ///
    /// # Type Parameters
    /// * `S` - A stream type that implements `AsyncRead`, `AsyncWrite`, and `Unpin`.
    ///
    /// # Arguments
    /// * `self` - Mutable reference to `TcpPerf` as `sample_size` might be updated (for Server).
    /// * `stream` - The mutable reference to the network stream.
    ///
    /// # Errors
    /// Returns `TcpPerfError` if any I/O error occurs or if the connection is closed prematurely.
    ///
    /// # Returns
    /// The total number of payload bytes successfully downloaded.
    pub async fn download<S>(&mut self, stream: &mut S) -> Result<u64, TcpPerfError>
    where
        S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + ?Sized,
    {
        let mut buf = vec![0u8; self.buf_size]; // Reusable buffer for reading chunks.
        let mut byte_counter: u64 = 0; // Counts downloaded payload bytes.
        let mut sample_size_buf: [u8; 8] = [0; 8]; // Buffer for the initial 8-byte size.
        
        // Phase 1: Read the total sample size (payload size).
        stream.read_exact(&mut sample_size_buf).await?;
        let received_sample_size = u64::from_le_bytes(sample_size_buf);

        // Handle zero sample size: if no data is expected, complete early.
        if received_sample_size == 0 {
            if self.mode == Mode::Server {
                self.sample_size = 0; // Server updates its sample_size for its subsequent upload.
            }
            if self.mode == Mode::Client {
                let mut pb = Self::_create_progress_bar(0);
                pb.finish_println("finished downloading (0 bytes)...\n");
            }
            return Ok(0); // 0 payload bytes downloaded.
        }
        
        let actual_payload_to_download = received_sample_size;

        if self.mode == Mode::Server {
            // Server updates its own sample_size based on what client indicated.
            // This is crucial for the server's subsequent upload to match client's expectation.
            self.sample_size = actual_payload_to_download;
        }

        // Phase 2: Read the actual payload data.
        match self.mode {
            Mode::Client => {
                let mut pb = Self::_create_progress_bar(actual_payload_to_download);
                while byte_counter < actual_payload_to_download {
                    let bytes_to_read = std::cmp::min(self.buf_size as u64, actual_payload_to_download - byte_counter) as usize;
                    match stream.read(&mut buf[..bytes_to_read]).await {
                        Ok(0) if byte_counter < actual_payload_to_download => return Err(TcpPerfError::ConnectionClosed), // Connection closed prematurely
                        Ok(0) => break, // Connection closed, all data received (byte_counter == actual_payload_to_download)
                        Ok(n) => {
                            byte_counter += n as u64;
                            pb.set(byte_counter);
                        }
                        Err(e) => return Err(TcpPerfError::IoError(e)),
                    }
                }
                // Ensure all expected bytes were received.
                if byte_counter < actual_payload_to_download {
                     return Err(TcpPerfError::ConnectionClosed);
                }
                pb.finish_println("finished downloading...\n");
            }
            Mode::Server => {
                while byte_counter < actual_payload_to_download {
                     let bytes_to_read = std::cmp::min(self.buf_size as u64, actual_payload_to_download - byte_counter) as usize;
                    match stream.read(&mut buf[..bytes_to_read]).await {
                        Ok(0) if byte_counter < actual_payload_to_download => return Err(TcpPerfError::ConnectionClosed), // Connection closed prematurely
                        Ok(0) => break, // Connection closed, all data received
                        Ok(n) => {
                            byte_counter += n as u64;
                        }
                        Err(e) => return Err(TcpPerfError::IoError(e)),
                    }
                }
                // Ensure all expected bytes were received.
                 if byte_counter < actual_payload_to_download {
                     return Err(TcpPerfError::ConnectionClosed);
                }
            }
        }
        Ok(byte_counter) 
    }
}
