# cansentinel

Monitors socketcan interfaces via netlink and restarts them when they enter bus-off state.

## Usage

```bash
cargo build --release
sudo ./target/release/cansentinel
```

Requires root to restart interfaces.