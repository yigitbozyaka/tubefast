# Tubefast

YouTube Music desktop client in Rust + egui (eframe 0.33, glow). No browser engine. Windows, macOS, Linux. Public repo `yigitbozyaka/tubefast`, MIT, Rust 1.90, edition 2024.

## Commands

```sh
cargo run -- --profile dev     # separate library and sign-in, use this to try changes
cargo run -- --selftest        # search, stream, decode, seek against YouTube, no window
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test   # CI runs exactly these, all must pass
```

`cargo test -- --include-ignored` also drives sign-in through a real headless Edge/Chrome. Skip it unless you touched `auth.rs`.

## Where to look

Big files: `ui.rs` (~2.9k lines), `ytm.rs` (~1.5k) and `app.rs` (~1.3k). Grep for the function you need. Do not read them whole.

| File | Owns |
|---|---|
| `src/app.rs` | `App` state, `Saved` (persisted), `Action` enum, `apply()` handles every action, `Event` from background threads, queue, shuffle, repeat |
| `src/ui.rs` | All drawing. `draw()` returns `Vec<Action>`, never mutates state. Colors and sizes are consts at the top |
| `src/ytm.rs` | YouTube API client, parsing, streams, lyrics (lrclib.net) |
| `clients.json` | YouTube client name/version/UA. Installed copies read it live from `master`, so a bad value breaks everyone. Run `--selftest` after changing it |
| `src/player.rs`, `src/opus.rs` | Download, decode, audio output |
| `src/hotkeys.rs` | Global hotkeys and their key combos |
| `src/media.rs` | Media keys, Windows overlay, macOS Now Playing, Linux MPRIS |
| `src/update.rs` | Updater: checksum-verified download, swap exe, restart with `--updated` |
| `src/auth.rs`, `src/art.rs`, `src/discord.rs` | Sign-in and cookie storage, artwork cache, Discord Rich Presence |

## How a feature is wired

1. UI: draw it in `ui.rs` in the style of existing custom controls (`icon_button`, `label`, `fit`, `paragraph`, consts like `ACCENT`, `FLOATING`). Do not use stock egui widgets.
2. Interaction: push a new `Action` variant; handle it in `App::apply`.
3. Slow work (network, disk, decoding): `self.background(|yt| Some(Event::...))`, then handle the `Event`. Never block the UI thread.
4. Persisted setting: add a field to `Saved` and to its `Default` impl. `#[serde(default)]` keeps old saves loading.
5. Tests live in a `mod tests` at the bottom of the same file. Add one small test for parsing or logic.

## Rules

- No code comments. Intent goes in names.
- No new dependency without a strong reason. Memory and startup time are features.
- `rustfmt.toml` sets `max_width = 140`.
- `master` is protected. Work on a branch, open a PR, PRs are squashed. Commit and PR titles are one plain user-facing sentence, e.g. "Loud songs no longer crackle".
- PR body follows `.github/pull_request_template.md`. Attach a screenshot if anything on screen changed.

## Release notes

`RELEASE_NOTES.md` holds the notes for the next release, one line per item. It is built into the app with `include_str!` and shown once in a card at the bottom left after an update. The release workflow also puts it at the top of the GitHub release page.

- Any change users can notice: add a short line to `RELEASE_NOTES.md` in the same PR.
- Lines are for users: short, plain, no PR numbers or code names.
- When the version is bumped, `RELEASE_NOTES.md` must describe only that version. Remove the previous release's lines.

## Releasing

1. Bump `version` in `Cargo.toml`, run `cargo build` so `Cargo.lock` updates, add new features to the README table, check `RELEASE_NOTES.md`.
2. PR titled `Tubefast X.Y.Z`, merge.
3. Push tag `vX.Y.Z` on `master`. `.github/workflows/release.yml` builds all platforms, writes `SHA256SUMS.txt`, publishes the release. The in-app updater picks it up from `releases/latest`.
