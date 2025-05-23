#[cfg(test)]
mod tests {
    use std::io::Cursor;
    // Note: AsyncReadExt, AsyncWriteExt are not directly used in test functions,
    // but they are used by the methods being tested (upload/download in TcpPerf).
    // The mock stream itself implements these traits.
    // use bytes::Bytes; // Not used

    use tcpperf::{Mode, TcpPerf, BUF_SIZE}; // Adjusted path and added BUF_SIZE

    // Mock TcpStream
    struct MockTcpStream {
        reader: Cursor<Vec<u8>>,
        writer: Cursor<Vec<u8>>,
        fail_read_after: Option<usize>, // Fail read after N bytes
        fail_write_after: Option<usize>, // Fail write after N bytes
        read_count: usize,
        write_count: usize,
    }

    impl MockTcpStream {
        fn new(read_data: Vec<u8>) -> Self {
            MockTcpStream {
                reader: Cursor::new(read_data),
                writer: Cursor::new(Vec::new()),
                fail_read_after: None,
                fail_write_after: None,
                read_count: 0,
                write_count: 0,
            }
        }

        #[allow(dead_code)]
        fn fail_read_after(mut self, bytes: usize) -> Self {
            self.fail_read_after = Some(bytes);
            self
        }

        #[allow(dead_code)]
        fn fail_write_after(mut self, bytes: usize) -> Self {
            self.fail_write_after = Some(bytes);
            self
        }
    }

    impl tokio::io::AsyncRead for MockTcpStream {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut tokio::io::ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            if let Some(fail_after) = self.fail_read_after {
                if self.read_count >= fail_after {
                    return std::task::Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "simulated read error",
                    )));
                }
            }
            let remaining_in_buf = buf.remaining();
            if remaining_in_buf == 0 { // Short-circuit if buffer has no space.
                return std::task::Poll::Ready(Ok(()));
            }
            let mut temp_buf = vec![0u8; remaining_in_buf];
            match std::io::Read::read(&mut self.reader, &mut temp_buf) {
                Ok(n) => {
                    buf.put_slice(&temp_buf[..n]);
                    self.read_count += n;
                    std::task::Poll::Ready(Ok(()))
                }
                Err(e) => std::task::Poll::Ready(Err(e)),
            }
        }
    }

    impl tokio::io::AsyncWrite for MockTcpStream {
        fn poll_write(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &[u8],
        ) -> std::task::Poll<Result<usize, std::io::Error>> {
            if let Some(fail_after) = self.fail_write_after {
                if self.write_count >= fail_after {
                    return std::task::Poll::Ready(Err(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        "simulated write error",
                    )));
                }
            }
            match std::io::Write::write(&mut self.writer, buf) {
                Ok(n) => {
                    self.write_count += n;
                    std::task::Poll::Ready(Ok(n))
                }
                Err(e) => std::task::Poll::Ready(Err(e)),
            }
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn test_upload_client() {
        let sample_size: u64 = 1024 * 10; // 10KB
        let mut mock_stream = MockTcpStream::new(vec![]);

        let tester = TcpPerf {
            mode: Mode::Client,
            sample_size,
            buf_size: BUF_SIZE, // Initialize with imported BUF_SIZE
        };

        let result = tester.upload(&mut mock_stream).await;
        assert!(result.is_ok());

        let written_data = mock_stream.writer.get_ref();
        let (size_bytes, data_bytes) = written_data.split_at(8);
        let written_sample_size = u64::from_le_bytes(size_bytes.try_into().unwrap());
        assert_eq!(written_sample_size, sample_size);
        assert_eq!(data_bytes.len() as u64, sample_size);

        for i in 0..data_bytes.len() {
            // The pattern repeats every BUF_SIZE block, based on the index within that block
            let local_idx = i % BUF_SIZE;
            let expected_byte = ((local_idx + 1) % u8::MAX as usize) as u8;
            assert_eq!(data_bytes[i], expected_byte, "Byte mismatch at global index {}, local_idx {}", i, local_idx);
        }
    }

    #[tokio::test]
    async fn test_upload_server() {
        let sample_size: u64 = 1024 * 5;
        let mut mock_stream = MockTcpStream::new(vec![]);
        let tester = TcpPerf { mode: Mode::Server, sample_size, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.upload(&mut mock_stream).await;
        assert!(result.is_ok());
        let written_data = mock_stream.writer.get_ref();
        let (size_bytes, data_bytes) = written_data.split_at(8);
        let written_sample_size = u64::from_le_bytes(size_bytes.try_into().unwrap());
        assert_eq!(written_sample_size, sample_size);
        assert_eq!(data_bytes.len() as u64, sample_size);
        for i in 0..data_bytes.len() {
            // The pattern repeats every BUF_SIZE block, based on the index within that block
            let local_idx = i % BUF_SIZE;
            let expected_byte = ((local_idx + 1) % u8::MAX as usize) as u8;
            assert_eq!(data_bytes[i], expected_byte, "Byte mismatch at global index {}, local_idx {}", i, local_idx);
        }
    }

    #[tokio::test]
    async fn test_download_client() {
        let sample_size: u64 = 1024 * 7;
        let mut server_sent_data = Vec::new();
        server_sent_data.extend_from_slice(&sample_size.to_le_bytes());
        let mut current_byte: u8 = 1;
        for _ in 0..sample_size {
            server_sent_data.push(current_byte);
            current_byte = current_byte.wrapping_add(1);
            if current_byte == 0 { current_byte = 1; }
        }
        let mut mock_stream = MockTcpStream::new(server_sent_data);
        let mut tester = TcpPerf { mode: Mode::Client, sample_size: 0, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.download(&mut mock_stream).await;
        assert!(result.is_ok());
        let bytes_downloaded = result.unwrap();
        assert_eq!(bytes_downloaded, sample_size);
        assert!(mock_stream.writer.get_ref().is_empty());
    }

    #[tokio::test]
    async fn test_download_server() {
        let sample_size: u64 = 1024 * 6;
        let mut client_sent_data = Vec::new();
        client_sent_data.extend_from_slice(&sample_size.to_le_bytes());
        let mut current_byte: u8 = 1;
        for _ in 0..sample_size {
            client_sent_data.push(current_byte);
            current_byte = current_byte.wrapping_add(1);
            if current_byte == 0 { current_byte = 1; }
        }
        let mut mock_stream = MockTcpStream::new(client_sent_data);
        let mut tester = TcpPerf { mode: Mode::Server, sample_size: 0, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.download(&mut mock_stream).await;
        assert!(result.is_ok());
        let bytes_downloaded = result.unwrap();
        assert_eq!(bytes_downloaded, sample_size);
        assert!(mock_stream.writer.get_ref().is_empty());
        assert_eq!(tester.sample_size, sample_size); // Check that server's sample_size was updated
    }

    #[tokio::test]
    async fn test_upload_client_zero_sample_size() {
        let sample_size: u64 = 0;
        let mut mock_stream = MockTcpStream::new(vec![]);
        let tester = TcpPerf { mode: Mode::Client, sample_size, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.upload(&mut mock_stream).await;
        assert!(result.is_ok());
        let written_data = mock_stream.writer.get_ref();
        assert_eq!(written_data.len(), 8);
        let written_sample_size = u64::from_le_bytes(written_data.as_slice().try_into().unwrap());
        assert_eq!(written_sample_size, sample_size);
    }

    #[tokio::test]
    async fn test_upload_server_zero_sample_size() {
        let sample_size: u64 = 0;
        let mut mock_stream = MockTcpStream::new(vec![]);
        let tester = TcpPerf { mode: Mode::Server, sample_size, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.upload(&mut mock_stream).await;
        assert!(result.is_ok());
        let written_data = mock_stream.writer.get_ref();
        assert_eq!(written_data.len(), 8);
        let written_sample_size = u64::from_le_bytes(written_data.as_slice().try_into().unwrap());
        assert_eq!(written_sample_size, sample_size);
    }

    #[tokio::test]
    async fn test_download_client_zero_sample_size() {
        let sample_size: u64 = 0;
        let mut server_sent_data = Vec::new();
        server_sent_data.extend_from_slice(&sample_size.to_le_bytes());
        let mut mock_stream = MockTcpStream::new(server_sent_data);
        let mut tester = TcpPerf { mode: Mode::Client, sample_size: 123, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.download(&mut mock_stream).await;
        assert!(result.is_ok());
        let bytes_downloaded = result.unwrap();
        assert_eq!(bytes_downloaded, sample_size);
        assert!(mock_stream.writer.get_ref().is_empty());
    }

    #[tokio::test]
    async fn test_download_server_zero_sample_size() {
        let sample_size: u64 = 0;
        let mut client_sent_data = Vec::new();
        client_sent_data.extend_from_slice(&sample_size.to_le_bytes());
        let mut mock_stream = MockTcpStream::new(client_sent_data);
        let mut tester = TcpPerf { mode: Mode::Server, sample_size: 456, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.download(&mut mock_stream).await;
        assert!(result.is_ok());
        let bytes_downloaded = result.unwrap();
        assert_eq!(bytes_downloaded, sample_size);
        assert!(mock_stream.writer.get_ref().is_empty());
        assert_eq!(tester.sample_size, sample_size); // Server's sample_size updated
    }

    #[tokio::test]
    async fn test_upload_client_write_error() {
        let sample_size: u64 = 1024 * 10;
        let mut mock_stream = MockTcpStream::new(vec![]).fail_write_after(1000 - 8); // Error during data write
        let tester = TcpPerf { mode: Mode::Client, sample_size, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.upload(&mut mock_stream).await;
        assert!(result.is_err());
    }
    
    #[tokio::test]
    async fn test_upload_client_write_error_on_size() {
        let sample_size: u64 = 1024 * 10;
        let mut mock_stream = MockTcpStream::new(vec![]).fail_write_after(0); // Error during sample_size write
        let tester = TcpPerf { mode: Mode::Client, sample_size, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.upload(&mut mock_stream).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_download_client_read_error_on_size() {
        let mut mock_stream = MockTcpStream::new(vec![1,2,3]).fail_read_after(0); // Not enough data for size, and error
        let mut tester = TcpPerf { mode: Mode::Client, sample_size: 0, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.download(&mut mock_stream).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_download_client_read_error_on_data() {
        let sample_size: u64 = 1024 * 10;
        let mut server_sent_data = Vec::new();
        server_sent_data.extend_from_slice(&sample_size.to_le_bytes());
        server_sent_data.extend_from_slice(&[1,2,3,4,5]); // Partial data
        let mut mock_stream = MockTcpStream::new(server_sent_data).fail_read_after(0); // Error during data read (after size read)
        let mut tester = TcpPerf { mode: Mode::Client, sample_size: 0, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.download(&mut mock_stream).await;
        assert!(result.is_err()); // Should be an error due to simulated read failure
    }

    #[tokio::test]
    async fn test_upload_server_write_error() {
        let sample_size: u64 = 1024 * 8;
        let mut mock_stream = MockTcpStream::new(vec![]).fail_write_after(500 - 8);
        let tester = TcpPerf { mode: Mode::Server, sample_size, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.upload(&mut mock_stream).await;
        assert!(result.is_err());
    }
    
    #[tokio::test]
    async fn test_upload_server_write_error_on_size() {
        let sample_size: u64 = 1024 * 8;
        let mut mock_stream = MockTcpStream::new(vec![]).fail_write_after(0);
        let tester = TcpPerf { mode: Mode::Server, sample_size, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.upload(&mut mock_stream).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_download_server_read_error_on_size() {
        let mut mock_stream = MockTcpStream::new(vec![1,2,3]).fail_read_after(0);
        let mut tester = TcpPerf { mode: Mode::Server, sample_size: 0, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.download(&mut mock_stream).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_download_server_read_error_on_data() {
        let sample_size: u64 = 1024 * 9;
        let mut client_sent_data = Vec::new();
        client_sent_data.extend_from_slice(&sample_size.to_le_bytes());
        client_sent_data.extend_from_slice(&[1,2,3,4,5]); // Partial data
        let mut mock_stream = MockTcpStream::new(client_sent_data).fail_read_after(0); // Error during data read
        let mut tester = TcpPerf { mode: Mode::Server, sample_size: 0, buf_size: BUF_SIZE }; // Initialize with imported BUF_SIZE
        let result = tester.download(&mut mock_stream).await;
        assert!(result.is_err());
    }
}
