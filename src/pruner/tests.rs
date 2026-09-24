use std::collections::HashSet;

use crate::error::IgError;
use crate::util::truncate;

use super::dates::{parse_to_datetime, validate_date_input};
use super::execute::{is_retryable_delete_error, map_delete_bool};
use super::filters::{
    first_protected_user, in_date_window, should_delete_dm, should_delete_media,
    thread_matches_kind,
};
use super::{drop_already_seen, ItemKind, Plan, PlannedDelete};
use crate::config::DmKind;
use crate::model::{Caption, DirectMessage, DirectThread, Media, UserShort};

fn media(pk: &str, author: &str, taken_at: i64) -> Media {
    Media {
        pk: pk.into(),
        id: Some(format!("{pk}_{author}")),
        code: Some("AbC".into()),
        taken_at,
        media_type: 1,
        product_type: None,
        caption: Some(Caption {
            text: "caption".into(),
        }),
        user: Some(UserShort {
            pk: author.into(),
            username: "me".into(),
            full_name: String::new(),
        }),
        image_versions2: None,
        thumbnail_url: None,
    }
}

fn dm(id: &str, user: &str, ts_us: i64) -> DirectMessage {
    DirectMessage {
        item_id: id.into(),
        id: None,
        user_id: Some(user.into()),
        timestamp: ts_us,
        item_type: Some("text".into()),
        text: Some("hello".into()),
    }
}

fn thread(id: &str, is_group: bool, user_ids: &[&str]) -> DirectThread {
    DirectThread {
        thread_id: id.into(),
        pk: None,
        thread_title: Some(format!("t-{id}")),
        is_group,
        users: user_ids
            .iter()
            .map(|u| UserShort {
                pk: (*u).into(),
                username: format!("u{u}"),
                full_name: String::new(),
            })
            .collect(),
        items: vec![],
        has_older: false,
        prev_cursor: None,
        oldest_cursor: None,
    }
}

#[test]
fn first_protected_user_returns_match() {
    let t = thread("1", false, &["a", "b"]);
    let protect: HashSet<&str> = ["b"].into_iter().collect();
    assert_eq!(first_protected_user(&t, &protect).as_deref(), Some("ub"));
}
#[test]
fn first_protected_user_matches_username_and_thread_id() {
    let t = thread("thread_999", false, &["a", "b"]);

    let protect_user: HashSet<&str> = ["@ub"].into_iter().collect();
    assert_eq!(
        first_protected_user(&t, &protect_user).as_deref(),
        Some("ub")
    );

    let protect_user_clean: HashSet<&str> = ["ub"].into_iter().collect();
    assert_eq!(
        first_protected_user(&t, &protect_user_clean).as_deref(),
        Some("ub")
    );

    let protect_thread: HashSet<&str> = ["thread_999"].into_iter().collect();
    assert_eq!(
        first_protected_user(&t, &protect_thread).as_deref(),
        Some("t-thread_999")
    );
}

#[test]
fn first_protected_user_none() {
    let t = thread("1", false, &["a"]);
    let protect: HashSet<&str> = HashSet::new();
    assert!(first_protected_user(&t, &protect).is_none());
}

#[test]
fn should_delete_filters_other_authors() {
    assert!(!should_delete_media(
        &media("1", "x", 1_700_000_000),
        "me",
        None,
        None
    ));
    assert!(!should_delete_dm(
        &dm("i", "x", 1_700_000_000_000_000),
        "me",
        None,
        None
    ));
}

#[test]
fn should_delete_allows_self() {
    assert!(should_delete_media(
        &media("1", "me", 1_700_000_000),
        "me",
        None,
        None
    ));
    assert!(should_delete_dm(
        &dm("i", "me", 1_700_000_000_000_000),
        "me",
        None,
        None
    ));
}

#[test]
fn should_delete_date_window() {
    let m = media("1", "me", 1_700_000_000);
    let before = parse_to_datetime("2024-01-01", crate::pruner::dates::DateBound::After).unwrap();
    let after = parse_to_datetime("2023-01-01", crate::pruner::dates::DateBound::After).unwrap();
    assert!(should_delete_media(&m, "me", Some(before), Some(after)));
    assert!(!should_delete_media(
        &m,
        "me",
        Some(parse_to_datetime("2023-01-01", crate::pruner::dates::DateBound::After).unwrap()),
        None
    ));
}

#[test]
fn unparseable_timestamp_fails_closed() {
    assert!(!should_delete_media(&media("1", "me", 0), "me", None, None));
    assert!(!in_date_window(None, None, None));
}

#[test]
fn thread_kind_filter() {
    let one = thread("1", false, &["a"]);
    let grp = thread("2", true, &["a", "b"]);
    assert!(thread_matches_kind(&one, DmKind::OneOnOne));
    assert!(!thread_matches_kind(&one, DmKind::Group));
    assert!(thread_matches_kind(&grp, DmKind::Group));
    assert!(thread_matches_kind(&grp, DmKind::All));
}

#[test]
fn planned_delete_to_record() {
    let p = PlannedDelete {
        target_id: "posts".into(),
        target_name: "Your posts".into(),
        item_id: "1_2".into(),
        kind: ItemKind::Media,
        preview: "hi".into(),
        author_id: "me".into(),
        author_name: "me".into(),
        content: "hi".into(),
        timestamp: "2024-01-01T00:00:00Z".into(),
        media_type: Some("photo".into()),
        attachments: vec![],
    };
    let r = p.to_record(crate::history::DeleteOutcome::DryRun);
    assert_eq!(r.target_id, "posts");
    assert_eq!(r.item_id, "1_2");
    assert_eq!(r.kind, "media");
    assert_eq!(r.outcome, crate::history::DeleteOutcome::DryRun);
}

#[test]
fn drop_already_seen_removes_ids() {
    let mut plan = Plan {
        entries: vec![
            PlannedDelete {
                target_id: "t1".into(),
                item_id: "m1".into(),
                ..Default::default()
            },
            PlannedDelete {
                target_id: "t1".into(),
                item_id: "m2".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let mut seen = HashSet::new();
    seen.insert(("t1".into(), "m1".into()));
    drop_already_seen(&mut plan, &seen);
    assert_eq!(plan.entries.len(), 1);
    assert_eq!(plan.entries[0].item_id, "m2");
}

#[test]
fn is_retryable_delete_error_only_5xx_and_http() {
    assert!(is_retryable_delete_error(&IgError::Api {
        status: 502,
        message: "bad".into()
    }));
    assert!(!is_retryable_delete_error(&IgError::Api {
        status: 400,
        message: "bad".into()
    }));
    assert!(!is_retryable_delete_error(&IgError::NotFound {
        resource: "x".into(),
        status: 404
    }));
    assert!(is_retryable_delete_error(&IgError::FeedbackRequired {
        message: "wait".into()
    }));
}

#[test]
fn truncate_caps_with_ellipsis() {
    let s = truncate(&"x".repeat(100), 10);
    assert_eq!(s.chars().count(), 11);
    assert!(s.ends_with('\u{2026}'));
}

#[test]
fn validate_date_input_accepts_common_formats() {
    assert!(validate_date_input("").unwrap().is_none());
    assert!(validate_date_input("2024-01-01").unwrap().is_some());
    assert!(validate_date_input("2024-01-01T12:00:00Z")
        .unwrap()
        .is_some());
    assert!(validate_date_input("2024-01-01 12:00:00")
        .unwrap()
        .is_some());
}

#[test]
fn validate_date_input_rejects_garbage() {
    assert!(validate_date_input("nope").is_err());
    assert!(validate_date_input("2024/01/01").is_err());
}

#[test]
fn from_media_builds_planned() {
    let m = media("99", "me", 1_700_000_000);
    let p = PlannedDelete::from_media(&m, "me");
    assert_eq!(p.target_id, "posts");
    assert_eq!(p.kind, ItemKind::Media);
    assert_eq!(p.item_id, "99_me");
    assert!(!p.preview.is_empty());
}

#[test]
fn from_dm_builds_planned() {
    let m = dm("item1", "me", 1_700_000_000_000_000);
    let p = PlannedDelete::from_dm(&m, "tid", "Alice", "me", "me");
    assert_eq!(p.target_id, "tid");
    assert_eq!(p.kind, ItemKind::Dm);
    assert_eq!(p.item_id, "item1");
    assert_eq!(p.content, "hello");
}

#[test]
fn map_delete_bool_false_is_err() {
    assert!(map_delete_bool(true, "x").is_ok());
    let err = map_delete_bool(false, "media delete").unwrap_err();
    assert!(err.to_string().contains("rejected"));

    assert!(!is_retryable_delete_error(&err));
}

#[test]
fn from_dm_stores_humanized_media_type() {
    let mut m = dm("item1", "me", 1_700_000_000_000_000);
    m.item_type = Some("voice_media".into());
    m.text = None;
    let p = PlannedDelete::from_dm(&m, "tid", "Alice", "me", "me");
    assert_eq!(p.media_type.as_deref(), Some("voice note"));
    assert_eq!(p.preview, "voice note");

    m.item_type = Some("action_log".into());
    let p = PlannedDelete::from_dm(&m, "tid", "Alice", "me", "me");
    assert_eq!(p.media_type.as_deref(), Some("system activity"));
}
