# pla-snapshot

Manual snapshot loop for PLA-003 M1-c: `SnapshotStore` copies the current
`.plasticity` file into `data/<doc>/snapshots/` with a sibling `.json` meta.

## Smoke example

Protocol smoke against a live bridge (subscribe_all + list_all, pretty JSON):

```sh
cargo run -p pla-snapshot --example smoke -- --url ws://127.0.0.1:8980 --doc mydoc.plasticity
```
