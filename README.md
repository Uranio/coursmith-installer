# Coursmith Setup

A small desktop app that gets a computer ready to create [Coursmith](https://coursmith.onrender.com) courses,
without typing commands:

1. **Choose** the AI subscription you have (Claude, ChatGPT, Gemini or another agent).
2. **Install**: the app finds or installs Node.js, the matching terminal agent and the `coursmith-course` skill.
3. **Connect** your Coursmith account from the browser.
4. **Ready**: one button opens the agent in your courses folder, with an example request to copy.

Everything is installed for the current user, under the home folder (`~/.coursmith`), with no administrator rights.

| Subscription | Agent installed | How |
|---|---|---|
| Claude Pro / Max | Claude Code | official install script |
| ChatGPT Plus / Pro / Business | Codex | npm package `@openai/codex` |
| Google AI Pro / Ultra / free | Antigravity CLI (`agy`) | official install script |
| other | OpenCode, Cursor CLI, pi, Gemini CLI (paid API key) | script or npm package |

## Download

Installers are attached to the [releases](../../releases): `.exe`/`.msi` for Windows, `.dmg` for macOS
(Apple Silicon and Intel), `.AppImage`/`.deb`/`.rpm` for Linux.

The installers are **not code-signed**, so the system warns the first time:

- **Windows**: "Windows protected your PC" → *More info* → *Run anyway*.
- **macOS**: right-click the app → *Open* → *Open*. If macOS says the app is damaged, run
  `xattr -cr "/Applications/Coursmith Setup.app"` in Terminal once.
- **Linux**: make the AppImage executable (`chmod +x`) and run it.

## What it does on your computer

- Downloads Node.js LTS from nodejs.org into `~/.coursmith/node` when no Node.js 20+ is found, after checking the
  file against the published SHA-256 checksum.
- Runs the agent's own installer (or `npm install --global --prefix ~/.coursmith/npm <package>`).
- Adds `~/.coursmith/node/bin` and `~/.coursmith/npm/bin` to the user's PATH (shell profile on macOS and Linux,
  user environment on Windows).
- Runs `npx --yes coursmith@latest install-skill --agent <agent>` and `coursmith login`.
- Creates the folder `~/Coursmith` for your courses.

To undo it: delete `~/.coursmith`, remove the "Added by Coursmith Setup" lines from your shell profile, and uninstall
the agent with its own instructions.

## Development

Requirements: Node.js 20+, Rust (stable) and the [Tauri prerequisites](https://tauri.app/start/prerequisites/).

```bash
npm install
npm run dev                         # run the app
cargo test --manifest-path src-tauri/Cargo.toml
# real download of Node.js and skill install into a throw-away home folder:
cargo test --manifest-path src-tauri/Cargo.toml -- --ignored --test-threads=1
npm run build                       # installers for the current system
```

The window is plain HTML, CSS and JavaScript in `src/` (no bundler). The work happens in `src-tauri/src/lib.rs`; the
list of agents and how each is installed is the `AGENTS` table at its top.

Pushing a tag `v*` builds all three systems with GitHub Actions and creates a draft release.
