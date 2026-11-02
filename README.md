# Interweave

A small GNOME app for Fedora that combines two or more internet connections (Ethernet, Wi-Fi, USB phone tether) to get more total download speed, and lets you cap download and upload speed per connection.

It's mainly for things like Steam, which download over many connections at once and can use all links together. A single download stream still goes over one link. Under the hood it uses `nmcli`, `ip route` and `tc`.

I also made it because I wanted to try Rust.

## Install

Download the `.rpm` from [Releases](https://github.com/NeDDy3z/Interweave/releases) and run:

```sh
sudo dnf install ./interweave-*.rpm
```

Build it yourself:

```sh
sudo dnf install cargo rust gtk4-devel libadwaita-devel
cargo install cargo-generate-rpm
cargo build --release && cargo generate-rpm
```

## License

Apache-2.0, see [LICENSE](LICENSE).
