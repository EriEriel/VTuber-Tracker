// `oshihub watch` — poll the backend for live VTubers and fire a desktop
// notification on each "went live" edge.
//
// Polling rather than a held-open connection (SSE was the original plan; see
// LIVE_DETECTION.md and Todo.md). The deciding argument is that this runs on
// a laptop: a pushed event fired while the lid is shut is gone forever,
// whereas a poll is self-healing — the next one returns current truth. It's
// also the only approach that's correct now that a local backend and the VPS
// can run at once, since Mongo is shared but an in-process broadcaster
// wouldn't be. The end-to-end latency cost is ~one interval, against a
// backend that already takes up to 5 minutes to notice a YouTube go-live.
//
// The backend hands out *state snapshots*; notifications need *edges*. That
// conversion is `apply()` below, which is pure and carries all the logic —
// the async loop underneath it is glue.

use std::collections::HashMap;
use std::io::IsTerminal;
use std::time::Duration;

use crate::config;
use crate::lock;
use crate::notify;
use crate::routes::{ApiError, LiveEntry, fetch_live_vtubers};
use crate::theme;

/// Give up on a poll well before the next one is due, so a hung request can
/// never let two cycles overlap.
const POLL_TIMEOUT_SECS: u64 = 20;
/// Consecutive failures tolerated before the delay starts growing.
const FAILURES_BEFORE_BACKOFF: u32 = 3;
const MAX_BACKOFF_SECS: u64 = 300;
/// Above this many simultaneous go-lives, send one summary instead of a
/// stack of individual popups.
const BURST_THRESHOLD: usize = 3;
/// A stream must be missing from this many *consecutive successful* polls
/// before it's considered over.
const MISSES_BEFORE_EVICTION: u8 = 2;

/// Identity of one live stream, stable across polls.
///
/// `externalId` is what makes this work: `stream.url` is
/// `twitch.tv/{login}` — the same string for every stream that channel ever
/// does — and `startTime` is rewritten whenever `markLive` re-upserts. Only
/// the external id is both unique per stream and immutable.
pub type StreamKey = String;

pub fn stream_key(entry: &LiveEntry) -> StreamKey {
    match &entry.stream.external_id {
        Some(id) => format!("{}:{}", entry.vtuber.id, id),
        // Backend predating the externalId field. Coarser — a back-to-back
        // second stream won't re-notify — but never wrong.
        None => entry.vtuber.id.clone(),
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum WatchState {
    /// Nothing believed about the world yet.
    Seeding,
    /// Live streams last seen, against consecutive-miss counts.
    Tracking(HashMap<StreamKey, u8>),
}

/// What the caller should do as a result of one poll.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// First successful poll: show what's already live, notify for none of it.
    Seeded(Vec<StreamKey>),
    /// A stream just started.
    WentLive(StreamKey),
    /// Several started at once (usually a suspend/resume catching up).
    BurstWentLive(Vec<StreamKey>),
}

/// Fold one poll result into the watch state.
///
/// Pure, and takes the poll *result* rather than doing the polling, which is
/// what makes every rule below testable without a network or a clock. It's
/// also the seam a future one-shot `oshihub check` would reuse: load state
/// from disk, call this, write it back.
pub fn apply(state: WatchState, poll: Result<&[LiveEntry], ()>) -> (WatchState, Vec<Action>) {
    let Ok(entries) = poll else {
        // A failed poll and an empty poll look identical in the data, but only
        // one of them means "nobody is live" — the same trap the backend's
        // scheduler guards against. Change nothing.
        return (state, Vec::new());
    };

    let current = dedupe_one_per_vtuber(entries);

    match state {
        // A failed *first* poll must not seed an empty world, or the first
        // success would notify for everyone already live. Staying in Seeding
        // is handled by the early return above; this arm is the success case.
        WatchState::Seeding => {
            let keys: Vec<StreamKey> = current.iter().map(|e| stream_key(e)).collect();
            let tracked = keys.iter().map(|k| (k.clone(), 0)).collect();
            (WatchState::Tracking(tracked), vec![Action::Seeded(keys)])
        }

        WatchState::Tracking(previous) => {
            let mut next: HashMap<StreamKey, u8> = HashMap::new();
            let mut fresh = Vec::new();

            for entry in &current {
                let key = stream_key(entry);
                if !previous.contains_key(&key) {
                    fresh.push(key.clone());
                }
                next.insert(key, 0);
            }

            // Anything absent this cycle is held for one more round rather
            // than dropped. Absorbs transient blips — including the window
            // live-state.ts documents, where markLive has closed the old
            // stream doc but not yet upserted the new one, so a poll landing
            // in between legitimately sees zero live docs.
            for (key, misses) in previous {
                if next.contains_key(&key) {
                    continue;
                }
                if misses + 1 < MISSES_BEFORE_EVICTION {
                    next.insert(key, misses + 1);
                }
            }

            let actions = match fresh.len() {
                0 => Vec::new(),
                n if n > BURST_THRESHOLD => vec![Action::BurstWentLive(fresh)],
                _ => fresh.into_iter().map(Action::WentLive).collect(),
            };

            (WatchState::Tracking(next), actions)
        }
    }
}

/// Collapse to one entry per VTuber, newest first.
///
/// Belt-and-braces: `markLive` makes two live docs for one VTuber
/// structurally impossible, and `/api/vtubers/live` dropped its dedup
/// workaround once that landed. A doc stuck live from before then would
/// otherwise produce two notifications for one person.
pub fn dedupe_one_per_vtuber(entries: &[LiveEntry]) -> Vec<LiveEntry> {
    let mut sorted: Vec<&LiveEntry> = entries.iter().collect();
    // Newest first; entries without a startTime sort last.
    sorted.sort_by(|a, b| b.stream.start_time.cmp(&a.stream.start_time));

    let mut seen = Vec::new();
    let mut out = Vec::new();
    for entry in sorted {
        if seen.contains(&entry.vtuber.id) {
            continue;
        }
        seen.push(entry.vtuber.id.clone());
        out.push(entry.clone());
    }
    out
}

/// Delay before the next poll after `failures` consecutive errors.
///
/// `checked_*` throughout: this counter runs unattended overnight, and
/// `2u64.pow(n)` panics on overflow in debug builds.
pub fn backoff_delay(failures: u32, base_secs: u64) -> Duration {
    if failures < FAILURES_BEFORE_BACKOFF {
        return Duration::from_secs(base_secs);
    }
    let doublings = failures - FAILURES_BEFORE_BACKOFF;
    let factor = 2u64.checked_pow(doublings.min(32)).unwrap_or(u64::MAX);
    let secs = base_secs.saturating_mul(factor).min(MAX_BACKOFF_SECS);
    Duration::from_secs(secs)
}

pub struct WatchOptions {
    pub interval_secs: Option<u64>,
    pub notify_existing: bool,
}

/// Resolve the poll gap: `--interval` beats anything configured, and the
/// floor applies either way.
fn resolve_interval(flag: Option<u64>, configured: u64) -> u64 {
    flag.unwrap_or(configured)
        .max(config::MIN_WATCH_INTERVAL_SECS)
}

pub async fn run(opts: WatchOptions) -> Result<(), Box<dyn std::error::Error>> {
    // Refuse to run alongside another watcher — two pollers means every
    // go-live notifies twice (the terminal-instance-next-to-the-systemd-
    // service trap). exit(1) rather than returning the error so the user
    // sees the Display message instead of main's Debug dump of it; non-zero
    // also keeps systemd's Restart=on-failure retrying until the other
    // instance stops, at which point the service takes over again.
    // `_lock` (not `_`): the guard must live until run() returns, or the
    // lock would release the moment it was acquired.
    let _lock = match lock::acquire() {
        Ok(guard) => Some(guard),
        Err(err @ lock::LockError::AlreadyRunning(_)) => {
            eprintln!("{err}");
            std::process::exit(1);
        }
        // An unwritable runtime dir shouldn't stop the watcher from doing
        // its actual job. Running unguarded is the lesser evil — but say so.
        Err(err) => {
            eprintln!("Warning: running without the single-instance lock: {err}");
            None
        }
    };

    let cfg = config::config();
    let interval_secs = resolve_interval(opts.interval_secs, cfg.watch_interval_secs);
    let notify_opts = notify::Options {
        timeout_ms: cfg.notify_timeout_ms,
    };

    println!(
        "Watching for live VTubers via {}",
        theme::url(config::api_url())
    );
    // Only mention Ctrl-C when there's someone at a keyboard. Under a systemd
    // user service this same line goes to the journal, where it's just wrong.
    let hint = if std::io::stdout().is_terminal() {
        format!("Polling every {interval_secs}s. Ctrl-C to stop.")
    } else {
        format!("Polling every {interval_secs}s.")
    };
    println!("{}", theme::muted(&hint));
    println!();

    // --notify-existing skips the quiet baseline so everyone currently live
    // notifies immediately. Mostly a way to see the real thing without
    // waiting for someone to go live.
    let mut state = if opts.notify_existing {
        WatchState::Tracking(HashMap::new())
    } else {
        WatchState::Seeding
    };

    let mut failures: u32 = 0;
    let mut last_error: Option<String> = None;

    loop {
        let poll = tokio::time::timeout(
            Duration::from_secs(POLL_TIMEOUT_SECS),
            fetch_live_vtubers(),
        )
        .await;

        let entries = match poll {
            Ok(Ok(entries)) => {
                failures = 0;
                last_error = None;
                Some(entries)
            }
            // No retry fixes a bad token, and a watcher that silently 401s
            // forever while you believe you're covered is worse than one that
            // stops. Exiting non-zero also stops systemd's Restart=on-failure
            // from looping on it.
            Ok(Err(ApiError::Unauthorized)) => return Err(ApiError::Unauthorized.into()),
            Ok(Err(err)) => {
                failures += 1;
                report_failure(&err.to_string(), &mut last_error);
                None
            }
            Err(_) => {
                failures += 1;
                report_failure(
                    &format!("poll timed out after {POLL_TIMEOUT_SECS}s"),
                    &mut last_error,
                );
                None
            }
        };

        let by_key: HashMap<StreamKey, LiveEntry> = entries
            .iter()
            .flatten()
            .map(|e| (stream_key(e), e.clone()))
            .collect();

        let (next_state, actions) = apply(state, entries.as_deref().ok_or(()));
        state = next_state;

        for action in actions {
            handle_action(action, &by_key, &notify_opts, cfg.notify_icons).await;
        }

        let delay = backoff_delay(failures, interval_secs);

        // Self-rescheduling rather than tokio::time::interval, whose default
        // MissedTickBehavior::Burst would fire a backlog of missed ticks
        // back-to-back after a slow poll or a laptop resume. Same reason
        // the backend's scheduler.ts avoids a bare setInterval.
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            _ = tokio::signal::ctrl_c() => {
                println!("\n{}", theme::muted("Stopped."));
                return Ok(());
            }
        }
    }
}

// First failure is loud, repeats of the same error are silent — a backend
// outage overnight shouldn't put a thousand identical lines in the journal.
fn report_failure(message: &str, last_error: &mut Option<String>) {
    if last_error.as_deref() != Some(message) {
        eprintln!("{}", theme::muted(&format!("Poll failed: {message}")));
        *last_error = Some(message.to_string());
    }
}

async fn handle_action(
    action: Action,
    by_key: &HashMap<StreamKey, LiveEntry>,
    notify_opts: &notify::Options,
    icons_enabled: bool,
) {
    match action {
        Action::Seeded(keys) => {
            if keys.is_empty() {
                println!("{}", theme::muted("Nothing live right now."));
            } else {
                println!(
                    "{}",
                    theme::heading(&format!(
                        "{} already live (no notifications for these):",
                        keys.len()
                    ))
                );
                for key in keys {
                    if let Some(entry) = by_key.get(&key) {
                        print_entry(entry);
                    }
                }
            }
            println!();
        }

        Action::WentLive(key) => {
            let Some(entry) = by_key.get(&key) else { return };
            print_entry(entry);
            println!("  {}", theme::url(&entry.stream.url));

            // A dashboard already on screen shows this go-live; a popup on
            // top of it is noise. Only the popup is skipped — the state fold
            // and the log line above already happened, so nothing re-fires
            // when the TUI closes and there's no gap to re-seed. Checked per
            // event, not per poll cycle: the TUI can open or close between
            // two entries of the same burst of actions.
            if lock::tui_is_present() {
                println!("  {}", theme::muted("(notification suppressed: a TUI is open)"));
                return;
            }

            let platform = match entry.vtuber.platform {
                crate::models::Platform::Youtube => "YouTube",
                crate::models::Platform::Twitch => "Twitch",
            };
            let icon = if icons_enabled {
                notify::cached_icon(&entry.vtuber.photo).await
            } else {
                None
            };
            notify::send(
                notify::Content {
                    summary: format!("{} is live on {platform}", entry.vtuber.english_name),
                    // Escaped because the daemon parses Pango markup in the
                    // body — verified: an unescaped "<b>" in a title renders
                    // as bold and the tags vanish.
                    body: notify::escape_markup(&entry.stream.title),
                    action_url: Some(entry.stream.url.clone()),
                    icon_path: icon,
                },
                notify_opts,
            )
            .await;
        }

        Action::BurstWentLive(keys) => {
            println!(
                "{}",
                theme::heading(&format!("{} VTubers went live", keys.len()))
            );
            let mut names = Vec::new();
            for key in &keys {
                if let Some(entry) = by_key.get(key) {
                    print_entry(entry);
                    names.push(entry.vtuber.english_name.clone());
                }
            }

            // Same TUI-open rule as WentLive above.
            if lock::tui_is_present() {
                println!("  {}", theme::muted("(notification suppressed: a TUI is open)"));
                return;
            }

            notify::send(
                notify::Content {
                    summary: format!("{} VTubers went live", keys.len()),
                    body: notify::escape_markup(&names.join(", ")),
                    action_url: None,
                    icon_path: None,
                },
                notify_opts,
            )
            .await;
        }
    }
}

// Deliberately no viuer thumbnail here, unlike `oshihub live`: that command
// draws into a static frame with manual cursor moves, which corrupts an
// append-only log — and under systemd there's no terminal at all.
fn print_entry(entry: &LiveEntry) {
    println!(
        "  {} {} - {}",
        theme::status_tag("live"),
        theme::name(&entry.vtuber.english_name),
        entry.stream.title
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::LiveStreamInfo;

    // Minimal LiveEntry built from the fields these rules actually read.
    fn entry(vtuber_id: &str, external_id: Option<&str>, start_time: Option<&str>) -> LiveEntry {
        let json = format!(
            r#"{{"_id":"{vtuber_id}","name":"n","englishName":"n","photo":"","platform":"twitch","source":"twitch_api","platformChannelId":"1","isTracked":true,"lastSyncedAt":null,"lastLiveSyncedAt":null,"lastStatsSyncedAt":null,"createdAt":"","updatedAt":"","__v":0}}"#
        );
        LiveEntry {
            vtuber: serde_json::from_str(&json).unwrap(),
            stream: LiveStreamInfo {
                title: "t".into(),
                // Constant per channel on Twitch — the whole reason the key
                // can't be built from this.
                url: "https://www.twitch.tv/same".into(),
                thumbnail_url: None,
                external_id: external_id.map(String::from),
                start_time: start_time.map(String::from),
            },
        }
    }

    fn tracking(keys: &[(&str, u8)]) -> WatchState {
        WatchState::Tracking(keys.iter().map(|(k, m)| (k.to_string(), *m)).collect())
    }

    #[test]
    fn key_combines_vtuber_and_external_id() {
        assert_eq!(stream_key(&entry("v1", Some("s1"), None)), "v1:s1");
    }

    // The regression test for the trap: same VTuber, same URL, different
    // streams. Keying on url would collapse these into one.
    #[test]
    fn two_twitch_streams_from_one_channel_get_different_keys() {
        let first = entry("v1", Some("s1"), None);
        let second = entry("v1", Some("s2"), None);
        assert_eq!(first.stream.url, second.stream.url);
        assert_ne!(stream_key(&first), stream_key(&second));
    }

    #[test]
    fn key_falls_back_to_vtuber_id_without_external_id() {
        assert_eq!(stream_key(&entry("v1", None, None)), "v1");
    }

    #[test]
    fn dedupe_keeps_only_the_newest_stream_per_vtuber() {
        let entries = vec![
            entry("v1", Some("old"), Some("2026-07-26T01:00:00.000Z")),
            entry("v1", Some("new"), Some("2026-07-26T03:00:00.000Z")),
            entry("v2", Some("other"), Some("2026-07-26T02:00:00.000Z")),
        ];
        let out = dedupe_one_per_vtuber(&entries);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].stream.external_id.as_deref(), Some("new"));
    }

    #[test]
    fn seeding_on_success_notifies_for_nothing() {
        let entries = vec![entry("v1", Some("s1"), None), entry("v2", Some("s2"), None)];
        let (state, actions) = apply(WatchState::Seeding, Ok(&entries));

        assert!(matches!(state, WatchState::Tracking(ref m) if m.len() == 2));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::Seeded(ref k) if k.len() == 2));
    }

    // A failed first poll must not seed an empty world — otherwise the first
    // success would look like "everyone just went live".
    #[test]
    fn seeding_stays_seeding_when_the_first_poll_fails() {
        let (state, actions) = apply(WatchState::Seeding, Err(()));
        assert_eq!(state, WatchState::Seeding);
        assert!(actions.is_empty());
    }

    #[test]
    fn failed_poll_leaves_tracking_state_untouched() {
        let before = tracking(&[("v1:s1", 0), ("v2:s2", 1)]);
        let (after, actions) = apply(before, Err(()));
        assert_eq!(after, tracking(&[("v1:s1", 0), ("v2:s2", 1)]));
        assert!(actions.is_empty());
    }

    #[test]
    fn new_stream_produces_one_went_live() {
        let entries = vec![entry("v1", Some("s1"), None)];
        let (_, actions) = apply(tracking(&[]), Ok(&entries));
        assert_eq!(actions, vec![Action::WentLive("v1:s1".into())]);
    }

    #[test]
    fn unchanged_live_set_produces_nothing() {
        let entries = vec![entry("v1", Some("s1"), None)];
        let (_, actions) = apply(tracking(&[("v1:s1", 0)]), Ok(&entries));
        assert!(actions.is_empty());
    }

    #[test]
    fn absent_once_is_held_not_evicted() {
        let (state, _) = apply(tracking(&[("v1:s1", 0)]), Ok(&[]));
        assert_eq!(state, tracking(&[("v1:s1", 1)]));
    }

    #[test]
    fn absent_twice_is_evicted() {
        let (state, _) = apply(tracking(&[("v1:s1", 1)]), Ok(&[]));
        assert_eq!(state, tracking(&[]));
    }

    // A one-cycle blip must not produce a duplicate notification when the
    // stream reappears.
    #[test]
    fn blip_then_return_does_not_renotify() {
        let entries = vec![entry("v1", Some("s1"), None)];
        let (held, _) = apply(tracking(&[("v1:s1", 0)]), Ok(&[]));
        let (_, actions) = apply(held, Ok(&entries));
        assert!(actions.is_empty());
    }

    // But a genuine restart after eviction should notify again.
    #[test]
    fn return_after_eviction_renotifies() {
        let entries = vec![entry("v1", Some("s1"), None)];
        let (evicted, _) = apply(tracking(&[("v1:s1", 1)]), Ok(&[]));
        let (_, actions) = apply(evicted, Ok(&entries));
        assert_eq!(actions, vec![Action::WentLive("v1:s1".into())]);
    }

    #[test]
    fn many_at_once_collapse_into_one_burst() {
        let entries: Vec<LiveEntry> = (0..4)
            .map(|i| entry(&format!("v{i}"), Some("s"), None))
            .collect();
        let (_, actions) = apply(tracking(&[]), Ok(&entries));
        assert_eq!(actions.len(), 1);
        assert!(matches!(actions[0], Action::BurstWentLive(ref k) if k.len() == 4));
    }

    #[test]
    fn at_the_threshold_they_stay_individual() {
        let entries: Vec<LiveEntry> = (0..BURST_THRESHOLD)
            .map(|i| entry(&format!("v{i}"), Some("s"), None))
            .collect();
        let (_, actions) = apply(tracking(&[]), Ok(&entries));
        assert_eq!(actions.len(), BURST_THRESHOLD);
        assert!(actions.iter().all(|a| matches!(a, Action::WentLive(_))));
    }

    #[test]
    fn backoff_holds_at_base_then_grows_and_caps() {
        assert_eq!(backoff_delay(0, 60), Duration::from_secs(60));
        assert_eq!(backoff_delay(2, 60), Duration::from_secs(60));
        assert_eq!(backoff_delay(3, 60), Duration::from_secs(60));
        assert_eq!(backoff_delay(4, 60), Duration::from_secs(120));
        assert_eq!(backoff_delay(10, 60), Duration::from_secs(MAX_BACKOFF_SECS));
    }

    // Runs unattended for hours; an arithmetic overflow panic here would be
    // a crash at 3am rather than a wrong number.
    #[test]
    fn backoff_does_not_panic_on_absurd_failure_counts() {
        assert_eq!(
            backoff_delay(u32::MAX, 60),
            Duration::from_secs(MAX_BACKOFF_SECS)
        );
    }

    #[test]
    fn interval_flag_beats_config_but_respects_the_floor() {
        assert_eq!(resolve_interval(Some(120), 60), 120);
        assert_eq!(resolve_interval(None, 60), 60);
        assert_eq!(
            resolve_interval(Some(1), 60),
            config::MIN_WATCH_INTERVAL_SECS
        );
    }
}
