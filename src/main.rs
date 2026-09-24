mod clipboard;
mod config;
mod events;
mod history;
mod instagram;
mod pruner;
mod tui;
mod util;

#[allow(unused_imports)]
pub(crate) use instagram::{client, device, error, model, ratelimit};
#[allow(unused_imports)]
pub(crate) use tui::theme;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use clap::Parser;

use crate::client::InstagramClient;
use crate::config::{Config, ScopeForm};

#[derive(Parser, Debug)]
#[command(
    name = "instagram-pruner",
    version,
    about = "Instagram content pruner (posts, DMs, likes, saved, comments, archived) via private mobile API"
)]
struct CliArgs {
    #[arg(long)]
    smoke: bool,

    #[arg(long, value_name = "THREAD_ID")]
    probe_thread: Option<String>,

    #[arg(long = "probe-activity")]
    probe_activity: bool,

    #[arg(long)]
    sessionid: Option<String>,

    #[arg(long)]
    scope: Option<String>,

    #[arg(long)]
    before: Option<String>,

    #[arg(long)]
    after: Option<String>,

    #[arg(long)]
    limit: Option<String>,

    #[arg(long = "throttle-ms")]
    throttle_ms: Option<String>,

    #[arg(long = "scan-throttle-ms")]
    scan_throttle_ms: Option<String>,

    #[arg(long = "dm-kind")]
    dm_kind: Option<String>,

    #[arg(long = "dry-run")]
    dry_run: bool,

    #[arg(long, value_name = "CLASSES")]
    content: Option<String>,

    #[arg(long = "hide-threads")]
    hide_threads: bool,

    #[arg(long, value_name = "IDS|FILE")]
    protect: Option<String>,
}

fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    init_tracing();

    let args = CliArgs::parse();
    let mut cfg = Config::from_env();

    if let Some(sessionid) = args.sessionid {
        cfg.sessionid = sessionid;
    }
    if let Some(scope_raw) = args.scope {
        let mode = crate::config::parse_scope(&scope_raw).map_err(|e| anyhow::anyhow!("{e}"))?;
        cfg.scope_form = ScopeForm::from_scope_mode(&mode);
        cfg.scope = mode;
    }
    if let Some(before) = args.before {
        cfg.before = before;
    }
    if let Some(after) = args.after {
        cfg.after = after;
    }
    if let Some(limit) = args.limit {
        cfg.limit = limit;
    }
    if let Some(throttle) = args.throttle_ms {
        cfg.throttle_ms = throttle;
    }
    if let Some(scan_throttle) = args.scan_throttle_ms {
        cfg.scan_throttle_ms = scan_throttle;
    }
    if let Some(dm_kind_raw) = args.dm_kind {
        cfg.dm_kind =
            crate::config::parse_dm_kind_value(&dm_kind_raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    if args.dry_run {
        cfg.dry_run = true;
    }
    if let Some(content_raw) = args.content {
        cfg.content =
            crate::config::parse_content_value(&content_raw).map_err(|e| anyhow::anyhow!("{e}"))?;
    }
    if args.hide_threads {
        cfg.hide_threads = true;
    }
    if let Some(spec) = args.protect {
        cfg.protect_users = parse_protect(&spec)?;
    }

    let runtime = tokio::runtime::Runtime::new()?;

    if args.smoke {
        return runtime.block_on(run_smoke(&cfg));
    }
    if let Some(thread_id) = args.probe_thread {
        return runtime.block_on(run_probe_thread(&cfg, &thread_id));
    }
    if args.probe_activity {
        return runtime.block_on(run_probe_activity(&cfg));
    }

    runtime.block_on(tui::run(cfg))
}

fn init_tracing() {
    let spec = match std::env::var("RUST_LOG").ok().filter(|s| !s.is_empty()) {
        Some(s) => s,
        None => return,
    };
    let filter = tracing_subscriber::EnvFilter::try_new(&spec)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn parse_protect(spec: &str) -> Result<Vec<String>> {
    let s = spec.trim();
    if s.is_empty() {
        bail!("--protect is empty");
    }
    let raw = if let Some(path) = s.strip_prefix('@') {
        if std::path::Path::new(path).is_file() {
            std::fs::read_to_string(path)
                .with_context(|| format!("--protect: cannot read file {path:?}"))?
        } else {
            s.to_string()
        }
    } else if std::path::Path::new(s).is_file() {
        std::fs::read_to_string(s).with_context(|| format!("--protect: cannot read {s:?}"))?
    } else {
        s.to_string()
    };

    let ids: Vec<String> = raw
        .split([',', '\n'])
        .flat_map(|line| line.split_whitespace())
        .map(str::trim)
        .filter(|x| !x.is_empty())
        .map(|x| x.trim_start_matches('@').to_string())
        .filter(|x| !x.is_empty())
        .collect();

    if ids.is_empty() {
        bail!("--protect: no user IDs found");
    }
    Ok(ids)
}

async fn cli_client(cfg: &Config) -> Result<InstagramClient> {
    let cancel = Arc::new(AtomicBool::new(false));
    let limiter = Arc::new(crate::ratelimit::RateLimiter::new(
        std::time::Duration::from_millis(350),
        cancel,
    ));
    InstagramClient::new(
        &cfg.sessionid,
        Some(&cfg.base_url),
        Some(&cfg.session_path),
        limiter,
    )
    .await
    .context("build instagram client")
}

async fn run_smoke(cfg: &Config) -> Result<()> {
    println!("[smoke] starting Instagram pruner smoke test");
    if cfg.sessionid.trim().is_empty() {
        bail!("INSTAGRAM_SESSIONID is empty. Set it in .env or pass --sessionid");
    }

    println!("[smoke] base url: {}", cfg.base_url);
    println!("[smoke] session file: {}", cfg.session_path);

    let client = cli_client(cfg).await?;

    println!("[smoke] authenticating / verifying session with get_current_user()...");
    let user_id = client.user_id();
    let username = client.username();
    let user_info = client.fetch_user_info(&user_id).await.ok();
    let full_name = user_info
        .as_ref()
        .map(|u| u.full_name.as_str())
        .unwrap_or("");

    println!(
        "[smoke] OK: authenticated as @{} (pk={}, name={:?})",
        username, user_id, full_name
    );

    println!("[smoke] fetching first page of user posts...");
    match client.user_medias_page(&user_id, None, 12).await {
        Ok((items, next)) => {
            println!(
                "[smoke] OK: retrieved {} posts (more_available={})",
                items.len(),
                next.is_some()
            );
            for (i, post) in items.iter().take(5).enumerate() {
                let caption = crate::util::truncate(&post.caption_text(), 50);
                println!("  post [{i}]: id={} caption={caption:?}", post.media_id());
            }
        }
        Err(e) => {
            println!("[smoke] warning: fetching feed posts failed: {e}");
        }
    }

    println!("[smoke] fetching first page of direct messages inbox...");
    match client.direct_inbox_page(None, 20).await {
        Ok((threads, _)) => {
            println!("[smoke] OK: retrieved {} threads", threads.len());
            for (i, thread) in threads.iter().take(5).enumerate() {
                let title = thread.display_name();
                let last_msg = thread
                    .items
                    .first()
                    .and_then(|it| it.text.as_ref())
                    .map(|s| crate::util::truncate(s, 40))
                    .unwrap_or_else(|| "(non-text item)".to_string());
                let protected = if cfg.protect_users.iter().any(|p| {
                    p == &thread.thread_id
                        || thread.users.iter().any(|u| &u.pk == p || &u.username == p)
                }) {
                    " [PROTECTED]"
                } else {
                    ""
                };
                println!(
                    "  thread [{i}]: id={} title={:?} last_msg={:?}{protected}",
                    thread.thread_id, title, last_msg
                );
            }
        }
        Err(e) => {
            println!("[smoke] warning: fetching inbox failed: {e}");
        }
    }

    if !cfg.protect_users.is_empty() {
        println!(
            "[smoke] protect list contains {} user/thread IDs",
            cfg.protect_users.len()
        );
    }

    println!("[smoke] smoke test finished successfully (nothing deleted)");
    Ok(())
}

async fn run_probe_thread(cfg: &Config, thread_id: &str) -> Result<()> {
    println!("[probe-thread] probing thread: {thread_id}");
    let client = cli_client(cfg).await?;

    let thread = client
        .direct_thread(thread_id, 20, None)
        .await
        .context("direct_thread")?;
    println!("[probe-thread] thread_id: {}", thread.thread_id);
    println!("[probe-thread] title: {}", thread.display_name());
    println!("[probe-thread] is_group: {}", thread.is_group);
    println!(
        "[probe-thread] users: {}",
        thread
            .users
            .iter()
            .map(|u| format!("@{} (pk={})", u.username, u.pk))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "[probe-thread] retrieved {} items in page",
        thread.items.len()
    );
    for (i, item) in thread.items.iter().take(10).enumerate() {
        let txt = item.text.as_deref().unwrap_or("(non-text)");
        println!(
            "  item [{i}]: id={} user={:?} type={:?} text={:?}",
            item.item_id, item.user_id, item.item_type, txt
        );
    }
    Ok(())
}

async fn run_probe_activity(cfg: &Config) -> Result<()> {
    if cfg.sessionid.trim().is_empty() {
        bail!("INSTAGRAM_SESSIONID is empty. Set it in .env or pass --sessionid");
    }
    let client = cli_client(cfg).await?;
    println!(
        "[probe-activity] authenticated as @{} (pk={})",
        client.username(),
        client.user_id()
    );

    let types = r#"["ALL_MEDIA_AUTO_COLLECTION","PRODUCT_AUTO_COLLECTION","MEDIA"]"#;
    match client
        .raw_get_json("collections/list/", &[("collection_types", types)])
        .await
    {
        Ok(v) => {
            println!("\n[probe-activity] collections/list/");
            if let Some(items) = v.get("items").and_then(|i| i.as_array()) {
                for item in items {
                    println!(
                        "  id={:?} name={:?} type={:?} count={:?}",
                        item.get("collection_id"),
                        item.get("collection_name"),
                        item.get("collection_type"),
                        item.get("collection_media_count"),
                    );
                }
            } else {
                println!(
                    "  (no items array) {}",
                    crate::util::truncate(&v.to_string(), 400)
                );
            }
        }
        Err(e) => println!("\n[probe-activity] collections/list/ failed: {e}"),
    }

    for (label, path) in [
        ("liked", "feed/liked/"),
        ("saved", "feed/saved/posts/"),
        ("archived", "feed/only_me_feed/"),
    ] {
        match client
            .raw_get_json(path, &[("include_igtv_preview", "false")])
            .await
        {
            Ok(v) => {
                let items = v.get("items").and_then(|i| i.as_array());
                let n = items.map(|a| a.len()).unwrap_or(0);
                println!("\n[probe-activity] {path} → {n} item(s)");
                if let Some(first) = items.and_then(|a| a.first()) {
                    let wrapped = first.get("media").is_some();
                    println!("  first item wrapped in `media` key: {wrapped}");
                    println!(
                        "  keys: {:?}",
                        first.as_object().map(|o| o.keys().collect::<Vec<_>>())
                    );
                }
                println!(
                    "  cursors: next_max_id={:?} max_id={:?} more_available={:?}",
                    v.get("next_max_id"),
                    v.get("max_id"),
                    v.get("more_available")
                );
            }
            Err(e) => println!("\n[probe-activity] {label} ({path}) failed: {e}"),
        }
    }

    let own = client
        .user_medias_page(&client.user_id(), None, 3)
        .await
        .map(|(items, _)| items)
        .unwrap_or_default();
    let probe_media = match own.first() {
        Some(media) => Some((media.media_id(), "own newest post")),
        None => match client.saved_medias_page(None).await {
            Ok((items, _)) => items.first().map(|m| (m.media_id(), "first saved post")),
            Err(_) => None,
        },
    };

    match probe_media {
        Some((media_id, source)) => {
            println!("\n[probe-activity] media/{media_id}/comments/ ({source})");
            match client.media_comments_page(&media_id, None).await {
                Ok((comments, next)) => {
                    println!(
                        "  parsed OK: {} comment(s), next={next:?}, {} authored by me",
                        comments.len(),
                        comments
                            .iter()
                            .filter(|c| c.author_id() == client.user_id())
                            .count()
                    );
                    for c in comments.iter().take(3) {
                        println!(
                            "    pk={} by @{} at={} text={:?}",
                            c.pk,
                            c.author_username(),
                            c.created_at,
                            crate::util::truncate(&c.text, 40)
                        );
                    }
                }
                Err(e) => println!("  failed: {e}"),
            }
        }
        None => println!("\n[probe-activity] no media available to probe comments on"),
    }

    println!("\n[probe-activity] done (nothing deleted)");
    Ok(())
}
