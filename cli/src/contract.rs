// Live contract tests: does the backend still send what this binary expects?
//
// The rest of the test suite deserializes *frozen captures* — real responses
// pasted in on the day they were captured (`models.rs`: 2026-06-27,
// `routes.rs`: 2026-07-26). Those prove the types could parse the backend as
// it was. They cannot notice the backend changing afterwards, and that failure
// mode is the bad one: `cargo test` stays green while every user's `oshihub`
// dies on a serde error.
//
// These tests close that gap by calling the same `routes.rs` functions the
// binary calls, against a real backend. Deliberately not a hand-written schema
// check — reusing the actual types is the whole point, since a second
// description of the shapes could itself drift.
//
// Every test is `#[ignore]`d so plain `cargo test` stays offline, as CLAUDE.md
// promises:
//
//     cargo test                  # unit tests only, no network
//     cargo test -- --ignored     # hits the backend from config.rs
//
// Target selection is `config::api_url()`, so the usual precedence applies —
// `OSHIHUB_API_URL=http://localhost:3000 cargo test -- --ignored` checks a
// local backend instead of production.
//
// See `docs/API_CONTRACT.md` for the shapes these assert, captured alongside.

use crate::routes;

/// Fails with a readable message rather than a bare `unwrap` panic — these run
/// against a live host, so "couldn't connect" and "shape changed" need to be
/// tellable apart at a glance.
macro_rules! ok_or_fail {
    ($expr:expr, $what:expr) => {
        match $expr {
            Ok(value) => value,
            Err(err) => panic!(
                "{} failed against {}: {err}\n\
                 (a Transport error means the backend is unreachable, not that the contract broke)",
                $what,
                crate::config::api_url()
            ),
        }
    };
}

/// `GET /api/vtubers` → `Vec<VtuberChannel>`.
///
/// The strictest check here by far: `VtuberChannel` declares `englishName`,
/// `photo`, `platform`, `source` and friends as non-`Option`, so this fails if
/// any single record is missing one. `org`/`suborg` are absent on most records
/// (only HoloDex knows about agencies) and must stay optional.
#[tokio::test]
#[ignore = "requires a reachable backend; run with --ignored"]
async fn vtuber_list_still_deserializes() {
    let vtubers = ok_or_fail!(routes::fetch_vtubers().await, "GET /api/vtubers");

    assert!(
        !vtubers.is_empty(),
        "backend returned zero VTubers — can't verify the contract against an empty list"
    );

    // Enum round-trip: a new `platform`/`source` value added backend-side
    // would already have failed deserialization above, but assert the parsed
    // values are self-consistent so the failure names the field.
    for v in &vtubers {
        assert!(!v.id.is_empty(), "empty _id on {}", v.name);
        assert!(
            !v.platform_channel_id.is_empty(),
            "empty platformChannelId on {}",
            v.name
        );
    }
}

/// `GET /api/vtubers/live` → `Vec<LiveEntry>`.
///
/// Passes when nobody is live (an empty array is a valid response), so this
/// checks the shape when it can and stays green when it can't. What it really
/// guards is `externalId`: `watch.rs` keys its go-live edge detection on it,
/// and losing it silently degrades every notification to once-per-channel-ever.
#[tokio::test]
#[ignore = "requires a reachable backend; run with --ignored"]
async fn live_list_still_deserializes() {
    let live = ok_or_fail!(routes::fetch_live_vtubers().await, "GET /api/vtubers/live");

    for entry in &live {
        assert!(
            entry.stream.external_id.is_some(),
            "live entry for {} has no externalId — `oshihub watch` would fall back to \
             a per-VTuber key and stop notifying for back-to-back streams",
            entry.vtuber.english_name
        );
        assert!(!entry.stream.url.is_empty());
    }
}

/// `GET /api/vtubers/:id` → `VtuberDetail`, plus the three dashboard endpoints.
///
/// One test rather than four: they all need a VTuber id, and fetching the list
/// four times to get one would just be slower.
#[tokio::test]
#[ignore = "requires a reachable backend; run with --ignored"]
async fn detail_and_stats_still_deserialize() {
    let vtubers = ok_or_fail!(routes::fetch_vtubers().await, "GET /api/vtubers");
    let subject = vtubers
        .first()
        .expect("backend returned zero VTubers — nothing to fetch detail for");
    let id = &subject.id;

    let detail = ok_or_fail!(
        routes::fetch_vtuber_detail(id).await,
        "GET /api/vtubers/:id"
    );
    // `VtuberDetail` deliberately models only streams and clips; serde ignores
    // `vtuber` and `snapshots`. Both arrays may legitimately be empty — a
    // freshly registered VTuber's initial sync is fire-and-forget.
    for stream in &detail.streams {
        assert!(
            matches!(
                stream.status.as_str(),
                "upcoming" | "live" | "ended" | "unknown"
            ),
            "unexpected Stream.status {:?} on {} — the enum grew a value the CLI's \
             theme::status_tag doesn't colour",
            stream.status,
            subject.english_name
        );
    }

    let freq = ok_or_fail!(
        routes::fetch_stream_frequency(id).await,
        "GET …/stats/stream-frequency"
    );
    // Dense and parallel is the contract the TUI's bar labels depend on.
    assert_eq!(
        freq.counts.len(),
        freq.starts.len(),
        "stream-frequency counts/starts lengths diverged ({} vs {}) — the dashboard \
         labels bars by index",
        freq.counts.len(),
        freq.starts.len()
    );

    let trend = ok_or_fail!(
        routes::fetch_subscriber_trend(id).await,
        "GET …/stats/subscriber-trend"
    );
    // Sparse by design: a missing day means "didn't sync", so only monotonic
    // day offsets are guaranteed, not one point per day.
    let mut previous: Option<u32> = None;
    for point in &trend.points {
        if let Some(prev) = previous {
            assert!(
                point.day > prev,
                "subscriber-trend points are not strictly ascending by day ({prev} then {})",
                point.day
            );
        }
        previous = Some(point.day);
    }

    let duration = ok_or_fail!(
        routes::fetch_duration_trend(id).await,
        "GET …/stats/duration-trend"
    );
    assert_eq!(
        duration.medians.len(),
        duration.counts.len(),
        "duration-trend medians/counts lengths diverged — `counts` is what lets the \
         mapper tell a pre-tracking bucket from a quiet one"
    );
    if !duration.starts.is_empty() {
        assert_eq!(
            duration.medians.len(),
            duration.starts.len(),
            "duration-trend medians/starts lengths diverged"
        );
    }
}

/// `GET /api/vtubers/:id/profile-url`.
///
/// Worth its own test because it is the one read endpoint that can fail for a
/// reason other than our own data: Twitch profile URLs need a live Helix call
/// to turn the stored numeric id back into a login name.
#[tokio::test]
#[ignore = "requires a reachable backend; run with --ignored"]
async fn profile_url_still_resolves() {
    let vtubers = ok_or_fail!(routes::fetch_vtubers().await, "GET /api/vtubers");
    let subject = vtubers.first().expect("backend returned zero VTubers");

    let url = ok_or_fail!(
        routes::fetch_profile_url(&subject.id).await,
        "GET …/profile-url"
    );
    assert!(
        url.starts_with("https://"),
        "profile-url returned {url:?}, which `open::that` can't act on"
    );
}
