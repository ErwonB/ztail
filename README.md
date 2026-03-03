# ztail — Zellij Log Tail Plugin

A Zellij plugin that monitors glob path patterns for new log files and automatically opens `tail -f` on each new file in a floating pane.

## Build

```bash
rustup target add wasm32-wasi
cargo build --release
```

The WASM binary will be at `target/wasm32-wasi/release/ztail.wasm`.

## Usage

Add the plugin to your Zellij layout (`.kdl` file):

```kdl
pane {
    plugin location="file:/path/to/ztail.wasm" {
        pattern_0 "/var/log/app-*.log"
        pattern_1 "/tmp/myservice-*.log"
        poll_interval "5"
    }
}
```

### Configuration

| Key            | Description                          | Default |
|----------------|--------------------------------------|---------|
| `pattern_N`    | Glob pattern for log files (0-indexed) | —       |
| `poll_interval`| Polling interval in seconds (float)  | `2.0`   |

## How It Works

1. Plugin loads, requests `FullHdAccess` + `RunCommands` permissions (one-time popup).
2. After permission grant, hides itself and runs silently in the background.
3. Every `poll_interval` seconds, expands each glob pattern against the host filesystem.
4. For each newly discovered file, opens a floating pane running `tail -f <file>`.

## Development

```bash
# Build and test with the included dev layout
cargo build --release
zellij --layout zellij.kdl

# Then create a test file:
touch /tmp/ztail-test-1.log
# A floating tail -f pane should appear within ~2 seconds

echo "hello world" >> /tmp/ztail-test-1.log
# Output appears in the floating pane
```
