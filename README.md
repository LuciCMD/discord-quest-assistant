# Discord Quest Assistant

Runs a stand-in for a game so Discord counts it as played for a quest. It's one exe for Windows: no
installer, no account, nothing to configure.

## How It Works

Discord knows a game by the name of its executable and the folders just above it, such as
`win64/game.exe`. It publishes that list. The app downloads the list, makes a copy of itself at that
path inside its own folder, and runs the copy for as long as you choose. The copy shows a small window
with the time left. When it ends or you stop it, the copy and its folder are deleted.

Some games, such as Marathon, aren't in Discord's list by file name; Discord finds them through
Steam instead. For those, turn on **Run From Steam Library**:

1. In Steam, start the game's download and pause it at 1-2%. Steam then creates the game's folder.
2. In the app, pick the game. It reads Steam's own files to find the folder and the game's exe.
3. Start it. If Steam has already put a partial file there, it's moved aside and put back afterwards.

## Using It

1. Type a game's name and pick it from the list.
2. Choose how long it runs. Most quests ask for 15 minutes; the default of 16 leaves a margin.
3. Press **Start**, keep the Discord desktop app open, and wait.

If the quest doesn't move, try another executable from the list. In Steam mode, try another launch
option, or enter the exe's path inside the game folder yourself (SteamDB and r/DiscordQuests list them).

## What It Touches

- **Its own folder**, `%LOCALAPPDATA%\Discord Quest Assistant\data`: the cached game list, the
  running copies, and a journal of every change.
- **In Steam mode only**, the one game folder Steam created. Every change is written to the journal
  first and undone afterwards, even after a crash or power cut: the next start puts things back. A
  file is deleted only if it is still byte for byte the app's own copy, and folders the app made are
  removed only once they're empty. An installed game's own files are never replaced.
- **The network**, only to download Discord's public list of detectable games
  (`discord.com/api/v9/applications/detectable`), once the saved copy is a day old or when you press
  **Refresh List**. There's no login and no token, and nothing is sent about you or your account.

## Building

Install Rust from [rustup.rs](https://rustup.rs), then run `build-release.bat`. The exe lands beside
it. For development:

```bash
cargo run
```

```bash
cargo test
```

```bash
cargo clippy --all-targets -- -D warnings
```

Some tests read real data or start real processes and are skipped by default; see the `#[ignore]`
notes in the code.

## A Word of Warning

Faking game activity goes against Discord's Terms of Service. Discord can warn, suspend or ban
accounts for it. Use it only on an account you're willing to risk.

The idea comes from Discord Quest Completer by ketch, which this replaces with a rewrite. The fonts are
Roboto and Cascadia Mono, under the SIL Open Font License (licences in `assets/fonts`).
