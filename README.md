<p align="center">
  <img src="assets/logo.svg" alt="Tubefast logo" width="88" height="88">
</p>

<h1 align="center">Tubefast</h1>

<p align="center"><strong>YouTube Music, native and fast.</strong><br>A lightweight desktop player for Windows, macOS and Linux with no browser engine inside.</p>

<p align="center">
  <a href="https://github.com/yigitbozyaka/tubefast/releases/latest"><strong>Download</strong></a> ·
  <a href="#what-you-get">Features</a> ·
  <a href="#build-from-source">Build from source</a> ·
  <a href="CONTRIBUTING.md">Contribute</a>
</p>

<p align="center">
  <a href="https://github.com/yigitbozyaka/tubefast/actions/workflows/ci.yml"><img src="https://github.com/yigitbozyaka/tubefast/actions/workflows/ci.yml/badge.svg" alt="CI status"></a>
  <a href="https://github.com/yigitbozyaka/tubefast/releases/latest"><img src="https://img.shields.io/github/v/release/yigitbozyaka/tubefast" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/yigitbozyaka/tubefast" alt="MIT license"></a>
</p>

![Tubefast playing a track, with the home feed in the middle and the queue on the right](docs/home.png)

Tubefast is a YouTube Music client written in Rust with [egui](https://github.com/emilk/egui). It talks to YouTube directly, decodes the audio itself and draws every pixel natively. There is no Chromium and no WebView underneath, which is why it starts instantly and stays small.

| | Measured |
|---|---|
| Window on screen | 0.1 to 0.3 seconds |
| Memory | about 100 MB while playing, about 60 MB of it private |
| CPU while playing | under 1 % |
| Download | one `.exe`, about 9 MB |

Numbers are from the release build on Windows 11. Memory is the working set a few seconds into playback, and it grows with window size and with how much artwork you have scrolled past. The first launch from a new folder used about 60 MB more on the test machine (NVIDIA driver), once.

## What you get

| | |
|---|---|
| **Search and browse** | Results appear as you type. Artist, album and playlist pages, with back and forward history. |
| **A queue that keeps going** | Play an album or a single song. When the queue runs out, a radio based on the last track continues it. Shuffle and repeat included. |
| **A queue you cannot lose** | Close the app and the queue, the track and the position are still there next time. Replace a queue you built by accident and **Undo** brings it back. Drag a row to reorder what plays next. |
| **Now playing and lyrics** | Click the cover or the title in the bar for a full-window view of the track. Its lyrics tab follows the song line by line when YouTube Music has timed lyrics, and shows plain text when it does not. |
| **Updates itself** | When a new version is out, Tubefast tells you and installs it with one click, then restarts with your queue where it was. |
| **Media keys** | Play, pause, next and previous keys work from any window, and the track shows up in the Windows media overlay in Now Playing on macOS and in the Linux desktop's media controls (MPRIS), with its cover and progress. |
| **Discord status** | Turn on **Show on Discord** under the gear at the bottom of the sidebar and your Discord profile shows the song you are listening to, with its artist, cover and progress. It is off until you turn it on. |
| **A home feed that knows you** | Signed out, the feed is built from what you played here. Signed in, you get your own YouTube Music feed and playlists. |
| **Your library, kept locally** | Liked songs, listening history and saved albums, playlists and artists live on your computer. |
| **Real covers** | Music videos show the album cover of the song instead of a video frame whenever YouTube knows which song it is. |
| **Keyboard first** | Shortcuts for everything that matters, plus accessible names on every control for screen readers. |
| **Profiles** | Run separate libraries and accounts side by side with `--profile`. |

## Install

**With [Scoop](https://scoop.sh)**, which shows no Windows warning:

```sh
scoop install https://github.com/yigitbozyaka/tubefast/releases/latest/download/tubefast.json
```

**By hand:**

1. Download `tubefast-windows-x64.zip` from the [latest release](https://github.com/yigitbozyaka/tubefast/releases/latest).
2. Right-click the zip, choose **Properties**, tick **Unblock** and press **OK**.
3. Unzip it anywhere and run `tubefast.exe`.

Step 2 is there because Tubefast is not code signed. Windows trusts a new program only after enough people have run it, so without that step the first launch shows "Windows protected your PC". If you see it, choose **More info**, then **Run anyway**.

**On macOS** (Apple silicon and Intel, macOS 11 or later):

1. Download `tubefast-macos-universal.zip` from the [latest release](https://github.com/yigitbozyaka/tubefast/releases/latest) and unzip it.
2. Move `Tubefast.app` to **Applications**.
3. Tubefast is not notarized by Apple, so clear the download flag once before the first launch:

```sh
xattr -dr com.apple.quarantine /Applications/Tubefast.app
```

**On Linux** (x86-64, any distribution with glibc 2.35 or newer, such as Ubuntu 22.04, Debian 12 or Fedora 36):

- **AppImage:** download `Tubefast-x86_64.AppImage` from the [latest release](https://github.com/yigitbozyaka/tubefast/releases/latest), make it executable and run it:

  ```sh
  chmod +x Tubefast-x86_64.AppImage
  ./Tubefast-x86_64.AppImage
  ```

- **Installed for your user, with a menu entry:** download `tubefast-linux-x64.tar.gz`, unpack it and run the installer. It puts Tubefast in `~/.local/bin` and adds it to your app menu. `./install.sh --uninstall` removes it again.

  ```sh
  tar -xzf tubefast-linux-x64.tar.gz
  tubefast/install.sh
  ```

You can check that a download was built from this repository by GitHub, not by anyone else:

```sh
gh attestation verify tubefast-windows-x64.zip --repo yigitbozyaka/tubefast
```

Each release also lists the SHA-256 of every download in `SHA256SUMS.txt`.

Windows and macOS are the platforms it is used on most. On macOS and Linux the session file is protected by file permissions instead of encryption, and a new version opens the download page instead of installing itself. Linux builds are new: the font fallback for non-Latin scripts is still missing there, and bug reports from Linux are very welcome. See [Contributing](CONTRIBUTING.md).

## Signing in

Everything except your personal feed and your playlists works without an account. To sign in, click **Sign in** at the bottom of the sidebar and pick one of two ways:

- **Continue in browser** opens a separate Edge or Chrome window. Sign in to Google there and the window closes by itself.
- **Paste your cookie** if the browser window is refused by Google. The dialog lists the three steps.

Your session never leaves your computer. It is stored in your user profile, encrypted with your Windows account (DPAPI) or, on macOS and Linux, in a file only your user can read, and is sent only to YouTube. Sign out removes it.

## Shortcuts

| Key | Action |
|---|---|
| `Space` | Play or pause |
| `Ctrl` + `K` or `/` | Jump to search |
| `Ctrl` + `→` / `Ctrl` + `←` | Next / previous track |
| `Ctrl` + `Z` | Bring back the queue you just replaced |
| `Esc` | Close the now playing view |
| Media keys | Play or pause, next, previous, from any window |
| `Alt` + `←` / `Alt` + `→` | Back / forward |
| `F5` or `Ctrl` + `R` | Reload the page |

Right-click a track, or use the **⋯** button on a row, for play next, add to queue, like, go to artist and go to album.

## Command line

```sh
tubefast "daft punk"                 # open with a search
tubefast --play "get lucky"          # search and play the first result
tubefast --profile work              # separate library, history and sign-in
tubefast --selftest                  # check search, streaming and decoding without opening a window
```

## Build from source

You need [Rust](https://rustup.rs) 1.90 or newer.

```sh
git clone https://github.com/yigitbozyaka/tubefast
cd tubefast
cargo run --release
```

On macOS, `cargo build --release && macos/bundle.sh` makes `target/Tubefast.app`, which is what gives Now Playing its name and icon.

On Linux, install the ALSA headers first (`sudo apt install libasound2-dev` on Debian and Ubuntu, `sudo dnf install alsa-lib-devel` on Fedora). `linux/package.sh` turns a release build into the same tarball the releases ship, and into an AppImage too when `appimagetool` is installed.

## How it works

- **Data** comes from the same internal API the YouTube Music website uses. Responses are parsed while streaming and the parts Tubefast never shows are skipped before they are allocated.
- **Audio** is fetched progressively, decoded with [Symphonia](https://github.com/pdeljanov/Symphonia) on its own thread and handed to [rodio](https://github.com/RustAudio/rodio). The interface never waits for the network. Tracks longer than about eight minutes keep a sliding 8 MB window in memory instead of the whole file, so an hour-long mix costs the same as a song.
- **The interface** is immediate-mode egui on OpenGL. It repaints only when something changes, so an idle window uses no CPU.

## Good to know

- **Tubefast is unofficial.** It is not affiliated with, endorsed by or sponsored by YouTube or Google. YouTube and YouTube Music are trademarks of Google LLC.
- **It can break.** YouTube changes its internal API without notice. The settings that usually need changing live in one small file, [`clients.json`](clients.json). When playback fails, Tubefast fetches the current copy from this repository and tries again, so most breakages are fixed for everyone with one commit and no new download. `tubefast --selftest` tells you which step failed.
- **What it talks to.** YouTube, and GitHub. At start it reads `clients.json` and checks whether a newer release exists. A new version is downloaded only when you click **Update**, from this repository's releases, and is installed only if it matches the SHA-256 published with the release. Copies installed with Scoop are updated with `scoop update tubefast` instead. With **Show on Discord** turned on, the title, artist, cover and position of the playing song are also handed to the Discord app on your computer. Nothing else about you is sent, and there is no telemetry.
- **If it crashes,** the reason is written to `crash.log` next to your data (`%APPDATA%\tubefast\data` on Windows, `~/.local/share/tubefast` on Linux). Attach it to your bug report.
- **Use it at your own risk.** YouTube's Terms of Service do not provide for third-party clients. Nobody can promise how YouTube treats accounts that use one. If that worries you, stay signed out.
- **Audio quality** is 128 kbps AAC for now.

## Not there yet

A tray icon, syncing likes back to your account, higher audio quality, a light theme, an installer, a notarized macOS build, and Flathub and AUR packages. Pull requests for any of these are welcome.

## Acknowledgements

Tubefast exists because of **[Spotifast](https://github.com/crmne/spotifast)** by [@crmne](https://github.com/crmne). Spotifast showed that a streaming client can be a small native app instead of a browser in disguise. Tubefast takes the same idea and the same choices (Rust, egui, no browser engine) and applies them to YouTube Music. If you use Spotify, go and star it.

Built on [egui](https://github.com/emilk/egui), [Symphonia](https://github.com/pdeljanov/Symphonia), [rodio](https://github.com/RustAudio/rodio) and [ureq](https://github.com/algesten/ureq). Icons are [Phosphor](https://phosphoricons.com) (MIT). Typefaces are [Onest](https://fonts.google.com/specimen/Onest) and [Big Shoulders Display](https://fonts.google.com/specimen/Big+Shoulders+Display) (SIL Open Font License, texts in `assets/fonts`). Client parameters are kept in step with the research done by the [yt-dlp](https://github.com/yt-dlp/yt-dlp) project.

## License

[MIT](LICENSE)
