# IOLinkWorkbench

Read-only IO-Link input inspection and CSV capture for Windows x64. This repository starts with a synthetic one-port sensor profile and a shared offline decoder for the CLI and desktop application.

## Install

Build with stable x64 Rust and the Visual Studio C++ Build Tools with a Windows SDK. Use `cargo build --locked --release`.

## Usage

See [sample settings](samples/simulator.toml) and the [synthetic IODD fragment](samples/sensor.xml).

## Maintainers

[votecoffee](https://github.com/votecoffee)

## Contributing

Run `cargo test --locked`. Keep private IODDs and captures in the ignored `.local/` directory.

## License

[MIT](LICENSE)
