# tcpperf

Rust based toy project for testing tcp network performance. 

## Installation

```bash
cargo run
```

Download prebuilt package from releases.
```bash
./tcpperf
```


## Usage

```bash
./tcpperf  --help
Usage: tcpperf [OPTIONS] <MODE> [ADDR]

Arguments:
  <MODE>  [possible values: server, client]
  [ADDR]  ip_addr:port e.g. 127.0.0.1:8080 [default: localhost:8080]

Options:
  -s, --sample-size <SAMPLE_SIZE>  Sample size in MByte [default: 100]
  -h, --help                       Print help
  -V, --version                    Print version
```

## Contributing

Pull requests are welcome. For major changes, please open an issue first
to discuss what you would like to change.

Please make sure to update tests as appropriate.

## License

[MIT](https://choosealicense.com/licenses/mit/)

