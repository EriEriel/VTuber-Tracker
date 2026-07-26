// Desktop notifications, via the `notify-send` binary from libnotify.
//
// Shelling out rather than talking D-Bus through a crate: `tokio` is already
// on `features = ["full"]`, so `tokio::process` costs nothing, and the
// project already shells out for `open::that()` in `jump_to`. The gap that
// would normally push you to `notify-rust` — click-to-open actions — is
// covered here by `-A` plus `--wait`, since the local daemon advertises the
// `actions` capability.
//
// Everything about the argument list below was checked against the running
// notification daemon (mako, via `GetCapabilities`) rather than assumed;
// three of the details are load-bearing and non-obvious. See `build_args`.

use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::theme;

/// mako renders icons at max 32px. Caching at 64 covers a HiDPI rescale
/// without storing 800px originals or making the daemon downscale on every
/// notification.
const ICON_SIZE: u32 = 64;

/// What a single notification should say.
pub struct Content {
    pub summary: String,
    pub body: String,
    /// Opened if the user clicks the notification. `None` disables the action.
    pub action_url: Option<String>,
    /// Absolute path to an already-downloaded icon. Never a URL —
    /// `notify-send -i` does not fetch over HTTP.
    pub icon_path: Option<String>,
}

pub struct Options {
    pub timeout_ms: u64,
}

// Notification delivery is best-effort: a machine without libnotify, or a
// session without a running daemon, must not take the watcher down with it.
// Once either of those is established, stop retrying and stop warning —
// otherwise a headless session logs the same failure every poll, forever.
static NOTIFIER_DISABLED: AtomicBool = AtomicBool::new(false);

pub fn is_disabled() -> bool {
    NOTIFIER_DISABLED.load(Ordering::Relaxed)
}

/// Escape the three characters Pango markup treats specially.
///
/// The daemon advertises `body-markup`, so an unescaped stream title
/// containing `&` or `<3` either renders wrong or has content silently
/// dropped — and VTuber titles contain both constantly. libnotify does not
/// escape for you.
///
/// Order matters: `&` must be replaced *first*. Doing `<` first would
/// introduce `&lt;` whose ampersand the later `&` pass would double-escape
/// into `&amp;lt;`.
pub fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Build the full `notify-send` argument list.
///
/// Pure, so the three traps below are covered by unit tests rather than by
/// firing real notifications and eyeballing them.
pub fn build_args(content: &Content, opts: &Options) -> Vec<String> {
    let mut args = Vec::new();

    // 1. `-a oshihub` is mandatory, not cosmetic. Without it the app name
    //    defaults to "notify-send", and omarchy's mako config carries a
    //    `[mode=do-not-disturb app-name=notify-send] invisible=false` rule —
    //    so every notification would punch straight through Do Not Disturb.
    //    Passing our own name also lets `[app-name=oshihub]` mako rules work.
    args.push("-a".into());
    args.push("oshihub".into());

    args.push("-t".into());
    args.push(opts.timeout_ms.to_string());

    // 2. Omit `-i` entirely when there's no icon. A generic themed fallback
    //    reads worse than no icon at all.
    if let Some(icon) = &content.icon_path {
        args.push("-i".into());
        args.push(icon.clone());
    }

    if content.action_url.is_some() {
        args.push("-A".into());
        args.push(format!("{ACTION_ID}=Watch"));
        // --wait keeps the process alive until the notification is dismissed
        // or actioned, and makes it print the chosen action id to stdout.
        // It has to be added here rather than appended by the caller: after
        // the `--` below it would be parsed as a third positional, and
        // notify-send rejects that with "Invalid number of options."
        args.push("--wait".into());
    }

    // 3. `--` terminates option parsing. There's no shell involved (so no
    //    injection risk), but notify-send parses args with GOption, which
    //    still treats a leading `-` in a positional as a flag — a stream
    //    title starting with "-" would otherwise break the invocation.
    args.push("--".into());
    args.push(content.summary.clone());
    args.push(content.body.clone());

    args
}

/// Action id echoed back on stdout by `notify-send --wait` when clicked.
pub const ACTION_ID: &str = "open";

/// FNV-1a. Eight lines instead of a hashing crate, and `DefaultHasher` is
/// explicitly documented as not stable across releases — which matters here,
/// since the output becomes a filename that has to stay valid between runs.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Cache filename for an avatar.
///
/// Keyed on the photo URL rather than the VTuber id, so a changed photo
/// yields a new filename and staleness self-corrects with no invalidation
/// logic. Hashing also guarantees the result can't contain `/` or `..` and
/// escape the cache directory.
pub fn cache_filename(photo_url: &str) -> String {
    format!("{:016x}.png", fnv1a(photo_url.as_bytes()))
}

fn cache_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("oshihub").join("avatars"))
}

/// Download, downscale and cache a VTuber avatar, returning a path suitable
/// for `notify-send -i` (which takes a local path or themed name — it will
/// not fetch an http URL).
///
/// Best-effort: every failure returns `None` and the notification simply
/// goes out without an icon.
pub async fn cached_icon(photo_url: &str) -> Option<String> {
    if photo_url.is_empty() {
        return None;
    }

    let dir = cache_dir()?;
    let path = dir.join(cache_filename(photo_url));
    if path.exists() {
        return Some(path.to_string_lossy().into_owned());
    }

    // external_client() deliberately carries no bearer token — this is a
    // third-party CDN, and the backend's shared secret has no business
    // going there.
    let res = crate::config::external_client()
        .get(photo_url)
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    let bytes = res.bytes().await.ok()?;

    // Re-encoding to PNG rather than storing the original also sidesteps
    // gdk-pixbuf loader availability — ggpht serves WebP, and whether the
    // daemon can decode that depends on an optional system package.
    let image = image::load_from_memory(&bytes).ok()?;
    let resized = image.thumbnail(ICON_SIZE, ICON_SIZE);

    std::fs::create_dir_all(&dir).ok()?;
    resized.save(&path).ok()?;

    Some(path.to_string_lossy().into_owned())
}

/// Fire one notification. Returns `false` if it couldn't be delivered.
///
/// Never returns an error: the caller is a long-running loop and a missing
/// notifier is a degraded mode, not a fatal one.
pub async fn send(content: Content, opts: &Options) -> bool {
    if is_disabled() {
        return false;
    }

    let args = build_args(&content, &opts);
    let wants_action = content.action_url.is_some();

    let mut command = tokio::process::Command::new("notify-send");
    command.args(&args);

    if wants_action {
        command.stdout(Stdio::piped());
    }

    let child = match command.spawn() {
        Ok(child) => child,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            // libnotify isn't installed at all.
            disable_with_warning(
                "notify-send not found — install libnotify for desktop notifications. \
                 Falling back to terminal output.",
            );
            return false;
        }
        Err(err) => {
            disable_with_warning(&format!("could not run notify-send ({err}) — falling back to terminal output."));
            return false;
        }
    };

    if !wants_action {
        return match child.wait_with_output().await {
            Ok(out) if out.status.success() => true,
            Ok(_) => {
                // notify-send exists but couldn't deliver — usually no
                // notification daemon on the session bus (mako not running,
                // or a tmux/SSH session with no desktop). A different failure
                // from "not installed", with a different fix.
                disable_with_warning(
                    "notify-send could not reach a notification daemon — falling back to \
                     terminal output.",
                );
                false
            }
            Err(err) => {
                disable_with_warning(&format!("notify-send failed ({err}) — falling back to terminal output."));
                false
            }
        };
    }

    // With an action attached the child outlives the notification, so it's
    // detached onto its own task rather than awaited here — otherwise the
    // poll loop would block until the user dealt with the popup.
    let url = content.action_url.clone().unwrap_or_default();
    tokio::spawn(async move {
        match child.wait_with_output().await {
            Ok(out) if out.status.success() => {
                if String::from_utf8_lossy(&out.stdout).trim() == ACTION_ID
                    && let Err(err) = open::that(&url)
                {
                    eprintln!("{}", theme::muted(&format!("could not open {url}: {err}")));
                }
            }
            Ok(_) => disable_with_warning(
                "notify-send could not reach a notification daemon — falling back to \
                 terminal output.",
            ),
            Err(err) => eprintln!("{}", theme::muted(&format!("notify-send failed: {err}"))),
        }
    });

    true
}

// Latch + warn exactly once. `swap` rather than store-then-check so two
// concurrent failures can't both print.
fn disable_with_warning(message: &str) {
    if !NOTIFIER_DISABLED.swap(true, Ordering::Relaxed) {
        eprintln!("{}", theme::muted(&format!("Warning: {message}")));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content(summary: &str, body: &str) -> Content {
        Content {
            summary: summary.to_string(),
            body: body.to_string(),
            action_url: None,
            icon_path: None,
        }
    }

    fn opts() -> Options {
        Options { timeout_ms: 10_000 }
    }

    #[test]
    fn escapes_pango_markup_characters() {
        assert_eq!(escape_markup("Tom & Jerry"), "Tom &amp; Jerry");
        assert_eq!(escape_markup("<3"), "&lt;3");
        assert_eq!(escape_markup("a & b <c>"), "a &amp; b &lt;c&gt;");
        assert_eq!(escape_markup("nothing special"), "nothing special");
    }

    // The ampersand has to be escaped before the angle brackets. If `<` went
    // first, the `&lt;` it produces would get its own `&` escaped again.
    #[test]
    fn escapes_ampersand_before_angle_brackets() {
        assert_eq!(escape_markup("<&>"), "&lt;&amp;&gt;");
    }

    // Escaping is deliberately NOT idempotent — running it twice double-
    // escapes. Asserted so nobody "fixes" a display bug by escaping again
    // somewhere up the call chain.
    #[test]
    fn escaping_is_not_idempotent() {
        assert_eq!(escape_markup(&escape_markup("&")), "&amp;amp;");
    }

    // Without -a, the app name defaults to "notify-send", which omarchy's
    // mako config explicitly lets through do-not-disturb.
    #[test]
    fn always_sets_the_app_name() {
        let args = build_args(&content("x", "y"), &opts());
        let pos = args.iter().position(|a| a == "-a").expect("-a is required");
        assert_eq!(args[pos + 1], "oshihub");
    }

    #[test]
    fn puts_double_dash_before_the_summary() {
        let args = build_args(&content("summary", "body"), &opts());
        let dash = args.iter().position(|a| a == "--").unwrap();
        let summary = args.iter().position(|a| a == "summary").unwrap();
        assert!(dash < summary);
    }

    // GOption would read a leading "-" as a flag; the "--" terminator is what
    // stops a stream title like "-1 hour stream" breaking the invocation.
    #[test]
    fn summary_starting_with_a_dash_lands_after_the_terminator() {
        let args = build_args(&content("-weird title", "body"), &opts());
        let dash = args.iter().position(|a| a == "--").unwrap();
        let summary = args.iter().position(|a| a == "-weird title").unwrap();
        assert!(dash < summary);
        assert_eq!(args.last().unwrap(), "body");
    }

    #[test]
    fn includes_icon_only_when_one_is_given() {
        let without = build_args(&content("x", "y"), &opts());
        assert!(!without.iter().any(|a| a == "-i"));

        let mut with_icon = content("x", "y");
        with_icon.icon_path = Some("/tmp/a.png".into());
        let args = build_args(&with_icon, &opts());
        let pos = args.iter().position(|a| a == "-i").unwrap();
        assert_eq!(args[pos + 1], "/tmp/a.png");
    }

    #[test]
    fn includes_action_only_when_a_url_is_given() {
        let without = build_args(&content("x", "y"), &opts());
        assert!(!without.iter().any(|a| a == "-A"));
        assert!(!without.iter().any(|a| a == "--wait"));

        let mut with_action = content("x", "y");
        with_action.action_url = Some("https://twitch.tv/x".into());
        let args = build_args(&with_action, &opts());
        let pos = args.iter().position(|a| a == "-A").unwrap();
        assert_eq!(args[pos + 1], format!("{ACTION_ID}=Watch"));
    }

    // Regression: --wait was originally appended after the arg list, which
    // put it past the `--` terminator. notify-send then counted it as a
    // third positional and refused the whole call with "Invalid number of
    // options." Every flag must precede the terminator.
    #[test]
    fn wait_flag_precedes_the_terminator() {
        let mut with_action = content("summary", "body");
        with_action.action_url = Some("https://twitch.tv/x".into());
        let args = build_args(&with_action, &opts());

        let wait = args.iter().position(|a| a == "--wait").unwrap();
        let terminator = args.iter().position(|a| a == "--").unwrap();
        assert!(wait < terminator, "--wait must come before `--`: {args:?}");

        // Exactly two positionals after the terminator.
        assert_eq!(args.len() - terminator - 1, 2);
    }

    #[test]
    fn passes_the_configured_timeout() {
        let args = build_args(&content("x", "y"), &Options { timeout_ms: 0 });
        let pos = args.iter().position(|a| a == "-t").unwrap();
        assert_eq!(args[pos + 1], "0");
    }

    #[test]
    fn cache_filename_is_stable_and_url_specific() {
        let a = "https://cdn.example.com/pekora.png";
        let b = "https://cdn.example.com/marine.png";
        assert_eq!(cache_filename(a), cache_filename(a));
        assert_ne!(cache_filename(a), cache_filename(b));
    }

    // The URL becomes a filename, so it must not be able to walk out of the
    // cache directory.
    #[test]
    fn cache_filename_cannot_escape_the_directory() {
        let nasty = cache_filename("https://x/../../../../etc/passwd");
        assert!(!nasty.contains('/'));
        assert!(!nasty.contains(".."));
        assert!(nasty.ends_with(".png"));
    }
}
