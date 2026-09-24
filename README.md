# Instagram Pruner

A terminal app for cleaning up your own Instagram posts, messages, and activity through Instagram's private mobile API.

> [!WARNING]
> Instagram can change this API without notice, and automated requests may trigger account checks. Start with a dry run and conservative request rates.

## Features

- Interactive terminal UI for posts, direct messages, likes, saved posts, comments, archived posts, and repost collections.
- Date, scope, item-count, and DM-kind filters, with support for protected threads.
- Dry-run mode and a local JSONL record of planned and completed changes.
- Throttled serial writes and concurrent read-only scans.

## Requirements

- Rust stable toolchain and Cargo.
- An Instagram session cookie from a logged-in browser. The app does not use password login.

## Build and run

```sh
cargo build --release
cp .env.example .env
```

Set `INSTAGRAM_SESSIONID` in `.env` to the value after `sessionid=`. The generated `session.json` also contains private session state.

Start the app:

```sh
cargo run --release
```

In **Settings**, leave **Dry run** enabled for the first run. Review the selected scope, filters, and targets before allowing deletes.

The read-only CLI options are `--smoke`, `--probe-thread <ID>`, and `--probe-activity`. See all options and overrides with:

```sh
cargo run -- --help
```

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `INSTAGRAM_SESSIONID` | — | Browser session cookie value |
| `INSTAGRAM_BASE_URL` | `https://i.instagram.com/api/v1` | Instagram API base URL |
| `INSTAGRAM_SESSION_PATH` | `session.json` | Private device and session state file |
| `INSTAGRAM_SCOPE` | `posts` | `none`, `posts`, `dms`, `all`, or `threads:<id>,<id>` |
| `INSTAGRAM_CONTENT` | — | Comma-separated activity classes: `likes`, `saved`, `comments`, `archived`, `reposts`; also `all` or `none` |
| `INSTAGRAM_DM_KIND` | `all` | DM filter: `all`, `1on1`, or `group` |
| `INSTAGRAM_BEFORE` / `INSTAGRAM_AFTER` | — | Date boundaries (`YYYY-MM-DD` or RFC 3339) |
| `INSTAGRAM_LIMIT` | `0` | Maximum planned items per run; `0` means no limit |
| `INSTAGRAM_THROTTLE_MS` | `2000` | Delay between write requests |
| `INSTAGRAM_SCAN_THROTTLE_MS` | `350` | Delay between scan requests |
| `INSTAGRAM_SCAN_CONCURRENCY` | `4` | Concurrent scan requests (`1`–`8`) |
| `INSTAGRAM_DRY_RUN` | `true` | Plan and archive without sending deletes |
| `INSTAGRAM_HIDE_THREADS` | `false` | Hide DM threads after a successful live run |
| `PRUNER_HISTORY_DIR` | `history` | Local JSONL record directory |

The `--protect` option and the Targets screen can protect DM threads. `INSTAGRAM_CONTENT` runs alongside the selected scope; with `INSTAGRAM_SCOPE=none`, it runs by itself. Set `RUST_LOG` to enable tracing. The client also honors standard proxy variables such as `HTTPS_PROXY`.

## Data and privacy

The app stores device and session state in `session.json` and records under `PRUNER_HISTORY_DIR` (default: `history/`). Treat both as private account data. Git ignores these paths. The local record is not an Instagram restore mechanism. Never add session files or history records to a commit.

## Known limitations

- Instagram has no documented repost-removal endpoint. Repost cleanup only scans saved collections whose name or type looks like a repost collection; review the dry-run plan carefully.
- Comment cleanup uses a private bulk-delete endpoint that has not been verified against a disposable comment. Leave it disabled until you can validate it safely.
- The OkHttp 5 profile currently bundled by `wreq-util` identifies itself as an unrelated app using an OkHttp 5 alpha build. This project keeps the Android-compatible OkHttp 4.12 transport profile instead of sending that mismatched fingerprint.

## Development

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

## License

GPL-3.0. See [`LICENSE`](LICENSE) for the full text.
