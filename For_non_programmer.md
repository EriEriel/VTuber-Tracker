# Setting up VTuber Tracker (no programming experience required)

## Table of contents

- [Why bother with this instead of just using the browser?](#why-bother-with-this-instead-of-just-using-the-browser)
- [What "the terminal" means](#what-the-terminal-means)
- [Checklist of accounts/keys you'll need](#checklist-of-accountskeys-youll-need)
- [Step 1 — Get the project files](#step-1--get-the-project-files)
- [Step 2 — Set up the database (MongoDB Atlas)](#step-2--set-up-the-database-mongodb-atlas)
- [Step 3 — Get your API keys](#step-3--get-your-api-keys)
  - [HoloDex](#holodex-optional-youtube-vtubers)
  - [YouTube / Google](#youtube--google-optional)
  - [Twitch](#twitch-optional)
- [Step 4 — Install Bun (runs the backend)](#step-4--install-bun-runs-the-backend)
- [Step 5 — Install Rust (builds the CLI)](#step-5--install-rust-builds-the-cli)
- [Step 6 — Configure and start the backend](#step-6--configure-and-start-the-backend)
- [Step 7 — Build and use the CLI](#step-7--build-and-use-the-cli)
- [Everyday use, after the first setup](#everyday-use-after-the-first-setup)
- [Advanced: instant live notifications with a dedicated server (optional)](#advanced-instant-live-notifications-with-a-dedicated-server-optional)
  - [Step A — Rent and prepare the server](#step-a--rent-and-prepare-the-server)
  - [Step B — Get the project onto the server](#step-b--get-the-project-onto-the-server)
  - [Step C — Get an HTTPS address for the server](#step-c--get-an-https-address-for-the-server)
  - [Step D — Install Caddy as a reverse proxy](#step-d--install-caddy-as-a-reverse-proxy-free-automatic-https)
  - [Step E — Configure and run the backend](#step-e--configure-and-run-the-backend)
  - [Step F — Open the firewall](#step-f--open-the-firewall)
  - [Step G — Point the CLI at your new server](#step-g--point-the-cli-at-your-new-server)
  - [Step H — Confirm it's working](#step-h--confirm-its-working)
  - [Step I — Run notifications as a background service](#step-i--run-notifications-as-a-background-service-linux-only-optional)
- [Troubleshooting](#troubleshooting)

---

## Why bother with this instead of just using the browser?

`oshihub` is a deliberately opinionated tool, and it's not for everyone. A few
honest reasons you might still want it:

- **You own the data and the database.** Nothing here depends on a
  third-party service staying online or keeping an API free — it's your
  MongoDB, your API keys, your rules.
- **If you already live in a terminal, it's faster.** Checking who's live or
  glancing at the streaming-frequency dashboard is a couple of keystrokes,
  not tabbing to a browser, waiting for a page to load, and clicking through
  to find the same information.
- **Live notifications are more reliable than platform bells.** YouTube's own
  bell notifications are known to arrive late or not at all sometimes.
  `oshihub watch` polls on an interval you choose, so you trade a small,
  known delay (your poll interval) for consistency — you don't just miss one.
- **It's lighter than a browser, measurably.** All the numbers below were
  read off the machine this was written on, not estimated. A background
  `oshihub watch` sits at about **15MB of memory**. The browser on the same
  machine was using **2.1GB** across its 22 processes — and even with every
  page closed, a browser still holds roughly **780MB** just to exist. So
  it's somewhere between **60× and 170× lighter**, depending on whether you
  compare against an empty browser or a real one with tabs open. Individual
  tabs on that machine ranged from 11MB for a light page to 616MB for a
  heavy one.
- **You end up owning your whole environment.** Setting this up means
  standing up a real database, running a backend service, and building a
  Rust binary yourself — good practice for understanding how the tools you
  use every day are actually built, even if you never touch the code.

None of that makes it a better *product* than just opening YouTube or Twitch
in a browser — it's a trade of a bit of setup effort for ownership, speed,
and lower resource use. If that trade doesn't appeal to you, this probably
isn't the guide you need.

---

This guide walks through getting the project running on your own computer,
step by step, assuming you've never used a terminal or written code before.
It covers **Windows, macOS, and Linux**.

The project has three pieces, and you need all three:

1. **A database** — where the VTuber data actually lives. We'll use a free
   cloud database (MongoDB Atlas) so you don't have to install and manage one
   yourself.
2. **The backend** — a small program that fetches data from YouTube/Twitch/
   HoloDex and saves it to the database. It runs quietly in a terminal window.
3. **The CLI (`oshihub`)** — the actual program you'll type commands into,
   like `oshihub live` to see who's streaming.

The core guide below runs everything on your own machine, refreshing data
with a manual `sync` command. An [optional advanced section](#advanced-instant-live-notifications-with-a-dedicated-server-optional)
further down covers moving the backend to a public server for instant
Twitch notifications and automatic YouTube polling — skip it on a first
read and come back once the basics are working.

---

## What "the terminal" means

You'll see the word **terminal** a lot below. It's a plain text window where
you type commands instead of clicking things. Every OS has one built in:

| OS | How to open it |
|---|---|
| Windows | Press `Win`, type `PowerShell`, press Enter |
| macOS | Press `Cmd+Space`, type `Terminal`, press Enter |
| Linux | Usually `Ctrl+Alt+T`, or search "Terminal" in your app menu |

When this guide says "run `something`", it means: type `something` into that
window and press Enter.

`★ Insight ─────────────────────────────────────`
A terminal isn't inherently more dangerous than any other app — it just has no
buttons, so nothing happens unless you type it. Copy-pasting commands from a
guide you trust (like this one) is normal practice, but it's also why guides
should never ask you to paste something you don't understand — always feel
free to ask what a command does before running it.
`─────────────────────────────────────────────────`

---

## Checklist of accounts/keys you'll need

You don't need all of these — only get keys for the platforms you actually
want to track.

| I want to track... | I need |
|---|---|
| YouTube VTubers that are listed on [holodex.net](https://holodex.net) | A HoloDex API key |
| YouTube VTubers/channels in general (or just stats) | A YouTube (Google) API key |
| Twitch streamers | A Twitch Client ID + Secret |
| Anything at all | A MongoDB database (always required) |

---

## Step 1 — Get the project files

If you were given a link to the GitHub page for this project:

1. Click the green **Code** button → **Download ZIP**.
2. Unzip it somewhere easy to find, like your Desktop or Documents folder.
   GitHub names the unzipped folder something like `VTuber-Tracker-main` —
   rename it to just `VTuber-Tracker`, since that's the name every command
   later in this guide assumes.

That folder is what this guide calls "the project folder" from now on. Inside
it you should see two folders named `backend` and `cli`.

---

## Step 2 — Set up the database (MongoDB Atlas)

Atlas is MongoDB's free hosted database service — no installation needed.

1. Go to [mongodb.com/atlas](https://www.mongodb.com/atlas) and sign up for a
   free account.
2. Create a new **free (M0) cluster** — the setup wizard defaults are fine,
   pick whatever region is closest to you.
3. When asked to create a database user, set a username and password.
   **Write these down** — you'll need them in a moment. Avoid special
   characters like `@` or `/` in the password; they cause problems later.
4. Under **Network Access**, add your current IP address (there's usually a
   button labeled "Add My Current IP Address"). Atlas blocks all connections
   by default until you allow yours.
5. Go to your cluster → **Connect** → **Drivers**, and copy the connection
   string. It looks like:
   ```
   mongodb+srv://yourusername:<password>@cluster0.xxxxx.mongodb.net/
   ```
6. Replace `<password>` with the actual password from step 3, and add a
   database name at the end, e.g. `.../vtuber-tracker?retryWrites=true...`
   (right after `.mongodb.net/`, before the `?`). Keep this full string
   somewhere — it's your `MONGODB_URI`.

`★ Insight ─────────────────────────────────────`
Atlas checks *where a connection is coming from* (your IP address) before it
even looks at your username/password — that's the Network Access step. If
your home internet assigns you a new IP address (routers sometimes do this
after a restart) and the backend suddenly can't connect, this allowlist is
the first place to check.
`─────────────────────────────────────────────────`

---

## Step 3 — Get your API keys

Skip any row below for a platform you don't care about.

### HoloDex (optional, YouTube VTubers)

1. Go to [holodex.net](https://holodex.net) and create an account.
2. Open your account settings — there's an API key shown there. Copy it.

### YouTube / Google (optional)

1. Go to [console.cloud.google.com](https://console.cloud.google.com) and
   sign in with a Google account.
2. Create a new project (top-left dropdown → "New Project").
3. In the search bar, search for **"YouTube Data API v3"** and click
   **Enable** on it.
4. Go to **APIs & Services → Credentials → Create Credentials → API key**.
   Copy the key it generates.

### Twitch (optional)

1. Go to [dev.twitch.tv/console](https://dev.twitch.tv/console) and log in
   with your Twitch account.
2. Go to **Applications → Register Your Application**.
3. Name it anything, set **OAuth Redirect URL** to `http://localhost:3000`,
   and pick **Category: Application Integration**.
4. Once created, copy the **Client ID**, then click **New Secret** to
   generate and copy a **Client Secret**.

---

## Step 4 — Install Bun (runs the backend)

⚠️ Important: you need the **canary** build of Bun, not the regular stable
release. The stable version currently crashes on startup due to a bug in one
of its dependencies. Canary is just as stable for our purposes — think of it
as "latest build" rather than "experimental."

**Windows** (PowerShell):
```powershell
powershell -c "irm bun.sh/install.ps1 | iex"
bun upgrade --canary
```

**macOS / Linux** (Terminal):
```sh
curl -fsSL https://bun.sh/install | bash
bun upgrade --canary
```

After installing, close and reopen your terminal, then verify it worked by
running:
```sh
bun -e "console.log(process.getBuiltinModule('v8').startupSnapshot.isBuildingSnapshot())"
```
This must print `false`. If it throws an error instead, you're on the stable
build — run `bun upgrade --canary` again.

---

## Step 5 — Install Rust (builds the CLI)

**All platforms:** go to [rustup.rs](https://rustup.rs) and follow the
one-line install command shown on the page (it detects your OS
automatically). Accept the default options when prompted. Restart your
terminal afterward.

Verify it worked:
```sh
cargo --version
```
You should see a version number, not an error.

---

## Step 6 — Configure and start the backend

1. In your terminal, navigate into the project folder. If you unzipped it to
   your Desktop, that's something like:
   ```sh
   cd Desktop/VTuber-Tracker/backend
   ```
   (`cd` means "change directory" — it moves your terminal into that folder.)

2. Install the backend's dependencies:
   ```sh
   bun install
   ```

3. Create a new file named exactly `.env` inside the `backend` folder — a
   plain text file, no extension beyond the dot. Any text editor works
   (Notepad, TextEdit, VS Code). Paste in only the lines for keys you
   actually have:
   ```
   MONGODB_URI=mongodb+srv://yourusername:yourpassword@cluster0.xxxxx.mongodb.net/vtuber-tracker
   HOLODEX_API_KEY=your-holodex-key
   YOUTUBE_API_KEY=your-youtube-key
   TWITCH_CLIENT_ID=your-twitch-client-id
   TWITCH_CLIENT_SECRET=your-twitch-secret
   ```
   `MONGODB_URI` is the only one that's always required.

4. Start the backend:
   ```sh
   bun run dev
   ```
   You should see `MongoDB connected successfully` printed. **Leave this
   terminal window open** — the backend needs to keep running the whole time
   you're using the CLI. Think of it like a local web server: closing the
   window shuts it down.

If something goes wrong here, see [Troubleshooting](#troubleshooting) below.

---

## Step 7 — Build and use the CLI

Open a **second, new terminal window** (keep the backend running in the
first one) and navigate to the `cli` folder:

```sh
cd Desktop/VTuber-Tracker/cli
cargo build --release
```

This takes a minute or two the first time — Rust is compiling the whole
program from source.

Optionally, put `oshihub` on your PATH so you can run it from anywhere
without typing the full path:
```sh
cargo install --path .
```

Now try it out:
```sh
oshihub create https://www.twitch.tv/tawffie
oshihub list
oshihub lookup tawffie
oshihub live
```

If you skipped `cargo install --path .`, run the built binary directly
instead — from inside the `cli` folder:
```sh
./target/release/oshihub list
```
(Windows: `.\target\release\oshihub.exe list`)

`★ Insight ─────────────────────────────────────`
The CLI talks to the backend over plain HTTP on `localhost:3000` by default —
that's why both terminal windows need to stay open. If you ever want the CLI
to talk to a backend running somewhere else, that's what the
`OSHIHUB_API_URL` setting (mentioned in the main README) is for; you won't
need it for a purely local setup like this one.
`─────────────────────────────────────────────────`

---

## Everyday use, after the first setup

Once everything above is done once, using it day-to-day is just:

1. Open a terminal, `cd` into `backend`, run `bun run dev`. Leave it running.
2. Open a second terminal and run whatever `oshihub` command you want
   (`oshihub live`, `oshihub lookup <name>`, etc.).
3. Data doesn't update itself in this basic setup — run `oshihub sync <name>`
   occasionally to pull fresh data for a specific VTuber.

---

## Advanced: instant live notifications with a dedicated server (optional)

Everything above lives on your own computer: live status only updates when
you run `oshihub sync`, and notifications only fire while your laptop is on
and `oshihub watch` is running in a terminal.

This section moves the backend onto a server that's on 24/7 with a real
public HTTPS address. Once that's in place:

- **Twitch pushes go-live events to your server the instant they happen**
  (a "webhook") instead of you having to check manually.
- **YouTube status refreshes automatically** every 5 minutes in the
  background (YouTube has no equivalent push mechanism, so polling is the
  best available — a few minutes of lag there is inherent, not a bug).
- **Notifications can run as a background service** tied to your login
  session, instead of a terminal window you have to keep open.

This is a real jump in complexity and responsibility: renting a server,
opening part of it to the public internet, managing TLS, and keeping a
security token safe. If the basic setup already does what you need, there's
no obligation to do this too.

### What you'll need

- A VPS (Virtual Private Server) from any provider — DigitalOcean, Linode,
  Hetzner Cloud, etc. — running Ubuntu or Debian, with a public IPv4 address.
  The cheapest tier from any of them (roughly $4–6/month) is more than
  enough; this backend is small.
- Comfort SSH'ing into a plain Linux box — exact commands are given below.
- Either your own domain name, or none at all (there's a free trick for
  that, in Step C).

### Step A — Rent and prepare the server

1. Create a small Ubuntu/Debian VPS through whichever provider you chose.
2. SSH into it: `ssh root@your-vps-ip`
3. Install Docker using its official convenience script:
   ```sh
   curl -fsSL https://get.docker.com | sh
   ```

### Step B — Get the project onto the server

Get the same project folder from Step 1 of the basic setup onto the server —
either `git clone` the repository (installing git first if needed:
`sudo apt install -y git`) if you have access to it, or upload the ZIP you
already downloaded with `scp local-file.zip root@your-vps-ip:~` and unzip it
there. Either way, make sure the resulting folder is named `VTuber-Tracker`
(a plain `git clone` already names it that; a ZIP upload needs the same
rename as in Step 1). Then:
```sh
cd VTuber-Tracker/backend
```

### Step C — Get an HTTPS address for the server

Twitch's webhook requires **HTTPS on port 443** — no exceptions, no
workarounds.

- **If you own a domain:** point an `A` record at your VPS's IP address
  (e.g. `oshihub.yourdomain.com → 1.2.3.4`), through whatever registrar or
  DNS host you use.
- **If you don't own one:** use [sslip.io](https://sslip.io) — a free
  service that turns any IP address into a working hostname with no signup
  and no DNS configuration:
  ```
  https://1-2-3-4.sslip.io      (dashes in place of the dots in your IP)
  ```
  This is a legitimate, widely used convenience service, not a workaround
  for anything shady — it just resolves that hostname straight back to the
  IP address embedded in it.

### Step D — Install Caddy as a reverse proxy (free automatic HTTPS)

Caddy gets you a valid Let's Encrypt certificate with almost no
configuration — it handles the "HTTPS on port 443" requirement for you.

```sh
sudo apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | sudo gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | sudo tee /etc/apt/sources.list.d/caddy-stable.list
sudo apt update && sudo apt install caddy
```

Edit `/etc/caddy/Caddyfile` (e.g. `sudo nano /etc/caddy/Caddyfile`) so it
contains just:
```
your-host.sslip.io {
    reverse_proxy 127.0.0.1:3001
}
```
using whichever hostname you settled on in Step C. Then apply it:
```sh
sudo systemctl reload caddy
```

### Step E — Configure and run the backend

First, generate two random secrets by running this command **twice** on the
server (once for each — they must be different values):
```sh
openssl rand -base64 32
```
Each run prints a random string to your terminal; copy it.

Then create `backend/.env` on the server with the same keys as the basic
setup (Step 3 above), **plus three more**:
```
API_TOKEN=paste-the-first-random-string-here
EVENTSUB_SECRET=paste-the-second-random-string-here
PUBLIC_URL=https://your-host.sslip.io
```
`API_TOKEN` isn't optional here — the container runs with
`NODE_ENV=production`, and the backend deliberately **refuses to start at
all** in production without one, rather than quietly serving an open API to
the whole internet.

Build and run the container, bound to `127.0.0.1` only so nothing but Caddy
(running on the same machine) can reach it directly:
```sh
docker build -t oshihub-backend .
docker run -d --name oshihub \
  --restart unless-stopped \
  -p 127.0.0.1:3001:3000 \
  -v $(pwd)/.env:/app/.env \
  oshihub-backend
docker logs -f --tail 30 oshihub
```
You should see `MongoDB connected successfully`. `Ctrl+C` to stop watching
the logs — the container keeps running.

### Step F — Open the firewall

In your VPS provider's dashboard, allow inbound traffic on **port 443**
(and briefly **80**, which Caddy uses to prove domain ownership to Let's
Encrypt). Leave port **3001 closed** to the outside world — it should only
ever be reached from Caddy on the same machine, which is why it was bound
to `127.0.0.1` above.

### Step G — Point the CLI at your new server

On your own computer (not the VPS), edit
`~/.config/oshihub/config.toml`:
```toml
api_url = "https://your-host.sslip.io"
api_token = "the same value you put in API_TOKEN above"
```
Every `oshihub` command now talks to your always-on server instead of a
local backend — you no longer need `bun run dev` running on your laptop at
all.

### Step H — Confirm it's working

```sh
curl -s -o /dev/null -w "%{http_code}\n" https://your-host.sslip.io/api/vtubers
```
This should print `401` — proof the server is publicly reachable *and*
correctly refusing a request with no token. Then run `oshihub list` from
your own machine; it should succeed, using the token from your config file.

### Step I — Run notifications as a background service (Linux only, optional)

Instead of keeping a terminal open running `oshihub watch`, turn it into a
systemd service that starts on login and survives you closing terminals.
Create `~/.config/systemd/user/oshihub-watch.service` **on your own
computer** (this runs locally — only the backend lives on the VPS):

```ini
[Unit]
Description=oshihub live VTuber notifications
PartOf=graphical-session.target
After=graphical-session.target

[Service]
Type=simple
ExecStart=%h/.cargo/bin/oshihub watch
Restart=on-failure
RestartSec=30

[Install]
WantedBy=graphical-session.target
```

Then:
```sh
systemctl --user daemon-reload
systemctl --user enable --now oshihub-watch
journalctl --user -u oshihub-watch -f   # watch its output
```

This needs `cargo install --path .` to have been run first (Step 7), since
it points at the installed binary rather than a build folder. On macOS or
Windows, which don't have systemd, the alternative is simply leaving
`oshihub watch` running in a terminal you don't close — not covered here.

Don't run this service **and** a manual `oshihub watch` in another terminal
at the same time; nothing stops both from running, and every notification
would fire twice.

### What changes after this

- Twitch streamers notify within seconds of going live.
- YouTube streamers are checked automatically every 5 minutes; that lag is
  the platform's limitation, not this project's.
- Your laptop only needs to run the small CLI notifier — the database
  connection and backend process live entirely on the server now.

### A note on responsibility

Once `PUBLIC_URL` is set, your backend is reachable by anyone who finds that
address, not just you. `API_TOKEN` and Twitch's signed webhook requests are
what keep it safe — don't remove either. Treat `.env`'s contents like
passwords: never commit the file, never paste it into a chat or a shared
document, and don't reuse the same `EVENTSUB_SECRET`/`API_TOKEN` anywhere
else.

---

## Troubleshooting

**"MongoServerError" or the backend never says "MongoDB connected"**
Almost always the Atlas Network Access allowlist (Step 2.4) — your IP
changed, or you never added it. Go back to Atlas → Network Access and add
your current IP again.

**Bun throws `ERR_NOT_IMPLEMENTED` on startup**
You're on stable Bun, not canary. Run `bun upgrade --canary` and try again.

**`cargo build` fails with a compiler error**
Run `rustup update` to make sure you have a recent Rust toolchain, then try
`cargo build --release` again.

**`oshihub` says it can't connect / connection refused**
The backend isn't running, or its terminal window was closed. Go back to the
first terminal and make sure `bun run dev` is still active and shows no
errors.

**I don't have an API key for a platform, can I still use the CLI?**
Yes — just leave that variable out of `.env` entirely. Only the features for
that platform won't work (e.g., no HoloDex key means YouTube VTubers get
looked up through the plain YouTube API path instead, if you have that key;
no Twitch keys means Twitch channels can't be tracked at all).

**(Advanced section) The health-check curl doesn't return `401`, or times out**
Check, in order: the VPS firewall actually allows port 443 (Step F); `docker
logs oshihub` shows `MongoDB connected successfully` and no crash; and
`sudo systemctl status caddy` shows it running with no certificate errors —
Caddy logs those to `journalctl -u caddy`. A `502` from Caddy means the
container itself isn't responding; a connection that never completes at all
usually means the firewall, not the app.

**(Advanced section) Twitch subscriptions never leave "pending"**
Means Twitch's initial verification request to `PUBLIC_URL/eventsub/callback`
never got a valid response — double check `PUBLIC_URL` in `.env` exactly
matches the hostname Caddy is serving (including `https://`, no trailing
slash), and that visiting that URL directly doesn't show a Caddy or TLS
error.
