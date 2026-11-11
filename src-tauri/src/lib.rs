//! Coursmith Setup: finds or installs Node.js and a terminal AI agent, installs the Coursmith skill for it with the
//! `coursmith` CLI and connects the user's Coursmith account. Everything is installed for the current user only,
//! under the home directory: no administrator rights are needed.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Emitter, Runtime};
use tauri_plugin_opener::OpenerExt;

const NODE_LINE: &str = "latest-v22.x";
const MIN_NODE_MAJOR: u32 = 20;
const CLI: &str = "coursmith@latest";

// ------------------------------------------------------------------------------------------------------- agents

/// How one agent is found and installed. `skill` is the name the coursmith CLI uses for it.
struct AgentDef {
    id: &'static str,
    name: &'static str,
    bin: &'static str,
    /// What the user needs to use it.
    plan: &'static str,
    /// Official install script for macOS and Linux.
    unix_script: Option<&'static str>,
    /// Official install script for Windows PowerShell.
    windows_script: Option<&'static str>,
    /// npm package to install when there is no script for this system.
    npm: Option<&'static str>,
    /// Where to send the user when it cannot be installed automatically.
    manual_url: &'static str,
}

const AGENTS: &[AgentDef] = &[
    AgentDef {
        id: "claude",
        name: "Claude Code",
        bin: "claude",
        plan: "Claude Pro or Max subscription",
        unix_script: Some("curl -fsSL https://claude.ai/install.sh | bash"),
        windows_script: Some("irm https://claude.ai/install.ps1 | iex"),
        npm: None,
        manual_url: "https://claude.com/claude-code",
    },
    AgentDef {
        id: "codex",
        name: "Codex",
        bin: "codex",
        plan: "ChatGPT Plus, Pro or Business subscription",
        unix_script: None,
        windows_script: None,
        npm: Some("@openai/codex"),
        manual_url: "https://developers.openai.com/codex/cli",
    },
    AgentDef {
        id: "antigravity",
        name: "Antigravity CLI",
        bin: "agy",
        plan: "Google account: Google AI Pro, Ultra or free",
        unix_script: Some("curl -fsSL https://antigravity.google/cli/install.sh | bash"),
        windows_script: Some("irm https://antigravity.google/cli/install.ps1 | iex"),
        npm: None,
        manual_url: "https://antigravity.google/docs/getting-started",
    },
    AgentDef {
        id: "opencode",
        name: "OpenCode",
        bin: "opencode",
        plan: "Any model provider or API key",
        unix_script: Some("curl -fsSL https://opencode.ai/install | bash"),
        windows_script: None,
        npm: Some("opencode-ai"),
        manual_url: "https://opencode.ai/docs",
    },
    AgentDef {
        id: "cursor",
        name: "Cursor CLI",
        bin: "cursor-agent",
        plan: "Cursor account",
        unix_script: Some("curl https://cursor.com/install -fsS | bash"),
        windows_script: None,
        npm: None,
        manual_url: "https://cursor.com/cli",
    },
    AgentDef {
        id: "pi",
        name: "pi",
        bin: "pi",
        plan: "Any model provider or API key",
        unix_script: None,
        windows_script: None,
        npm: Some("@mariozechner/pi-coding-agent"),
        manual_url: "https://github.com/badlogic/pi-mono",
    },
    AgentDef {
        id: "gemini",
        name: "Gemini CLI",
        bin: "gemini",
        plan: "Paid Gemini API key (not Google AI Pro: use Antigravity)",
        unix_script: None,
        windows_script: None,
        npm: Some("@google/gemini-cli"),
        manual_url: "https://geminicli.com",
    },
];

fn agent(id: &str) -> Result<&'static AgentDef, String> {
    AGENTS.iter().find(|a| a.id == id).ok_or_else(|| format!("Unknown agent: {id}"))
}

// -------------------------------------------------------------------------------------------------------- paths

fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    PathBuf::from(std::env::var_os(var).unwrap_or_default())
}

/// Everything this app installs lives here.
fn base_dir() -> PathBuf {
    home().join(".coursmith")
}

fn node_dir() -> PathBuf {
    base_dir().join("node")
}

/// Folder holding `node` and `npm` of the private Node.js.
fn node_bin_dir() -> PathBuf {
    if cfg!(windows) {
        node_dir()
    } else {
        node_dir().join("bin")
    }
}

/// npm packages (agents) are installed with this prefix, so no global npm folder or sudo is involved.
fn npm_prefix() -> PathBuf {
    base_dir().join("npm")
}

fn npm_bin_dir() -> PathBuf {
    if cfg!(windows) {
        npm_prefix()
    } else {
        npm_prefix().join("bin")
    }
}

fn workspace_dir() -> PathBuf {
    home().join("Coursmith")
}

/// Folders where agents and Node.js usually are. A desktop app does not get the PATH of the user's shell
/// (on macOS it is almost empty), so they are searched explicitly.
fn extra_path_dirs() -> Vec<PathBuf> {
    let h = home();
    let mut dirs = vec![node_bin_dir(), npm_bin_dir(), h.join(".local").join("bin")];
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            dirs.push(PathBuf::from(appdata).join("npm"));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let local = PathBuf::from(local);
            dirs.push(local.join("agy").join("bin"));
            dirs.push(local.join("Programs").join("nodejs"));
        }
        dirs.push(PathBuf::from(r"C:\Program Files\nodejs"));
    } else {
        for d in [".claude/local", ".claude/bin", ".opencode/bin", ".npm-global/bin", ".bun/bin", ".volta/bin", ".cargo/bin"] {
            dirs.push(h.join(d));
        }
        for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin", "/snap/bin"] {
            dirs.push(PathBuf::from(d));
        }
    }
    dirs
}

fn full_path() -> std::ffi::OsString {
    let mut dirs = extra_path_dirs();
    if let Some(current) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&current));
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

/// Finds a program in the usual folders and in PATH. On Windows npm installs `.cmd` launchers.
fn find_program(name: &str) -> Option<PathBuf> {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat"] } else { &[""] };
    for dir in std::env::split_paths(&full_path()) {
        for ext in exts {
            let candidate = dir.join(format!("{name}{ext}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

// ----------------------------------------------------------------------------------------------------- processes

fn hide_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// A command for `program`, with the extended PATH. `.cmd` launchers run through cmd.exe.
fn command(program: &Path) -> Command {
    let is_script = program
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
        .unwrap_or(false);
    let mut cmd = if cfg!(windows) && is_script {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(program);
        c
    } else {
        Command::new(program)
    };
    cmd.env("PATH", full_path()).stdin(Stdio::null());
    hide_console(&mut cmd);
    cmd
}

/// Runs a shell snippet: bash on macOS and Linux, PowerShell on Windows.
fn shell(script: &str) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("powershell");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script]);
        c
    } else {
        let mut c = Command::new("bash");
        c.args(["-c", script]);
        c
    };
    cmd.env("PATH", full_path()).stdin(Stdio::null());
    hide_console(&mut cmd);
    cmd
}

fn log<R: Runtime>(app: &AppHandle<R>, line: impl Into<String>) {
    let _ = app.emit("log", line.into());
}

/// Runs a command to the end, sending its output to the window line by line. Returns what it printed on stdout.
fn exec<R: Runtime>(app: &AppHandle<R>, mut cmd: Command, what: &str) -> Result<String, String> {
    log(app, format!("› {what}"));
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{what}: could not start ({e})."))?;

    let stderr = child.stderr.take().expect("piped stderr");
    let app_err = app.clone();
    let err_thread = std::thread::spawn(move || {
        let mut tail = Vec::new();
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            log(&app_err, line.clone());
            tail.push(line);
            if tail.len() > 12 {
                tail.remove(0);
            }
        }
        tail
    });

    let mut out = String::new();
    for line in BufReader::new(child.stdout.take().expect("piped stdout")).lines().map_while(Result::ok) {
        log(app, line.clone());
        out.push_str(&line);
        out.push('\n');
    }
    let status = child.wait().map_err(|e| format!("{what}: {e}"))?;
    let err_tail = err_thread.join().unwrap_or_default();
    if status.success() {
        Ok(out)
    } else {
        let detail = if err_tail.is_empty() { out.lines().rev().take(6).collect::<Vec<_>>().join(" ") } else { err_tail.join(" ") };
        Err(format!("{what} failed. {}", detail.trim()))
    }
}

/// Quietly runs a command and returns its stdout (for version checks).
fn capture(mut cmd: Command) -> Option<String> {
    let out = cmd.stdout(Stdio::piped()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

// ------------------------------------------------------------------------------------------------------- Node.js

/// The usable Node.js, if any: its version and whether it is new enough.
fn node_version() -> Option<(String, bool)> {
    let node = find_program("node")?;
    let version = capture({
        let mut c = command(&node);
        c.arg("--version");
        c
    })?;
    let major = version.trim_start_matches('v').split('.').next()?.parse::<u32>().ok()?;
    Some((version, major >= MIN_NODE_MAJOR))
}

fn node_archive_suffix() -> Result<&'static str, String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("linux-x64.tar.gz"),
        ("linux", "aarch64") => Ok("linux-arm64.tar.gz"),
        ("macos", "x86_64") => Ok("darwin-x64.tar.gz"),
        ("macos", "aarch64") => Ok("darwin-arm64.tar.gz"),
        ("windows", "x86_64") => Ok("win-x64.zip"),
        ("windows", "aarch64") => Ok("win-arm64.zip"),
        (os, arch) => Err(format!("Node.js has no download for {os} on {arch}. Install it from https://nodejs.org and run setup again.")),
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// curl ships with macOS, Windows 10+ and practically every Linux desktop.
fn curl(args: &[&str]) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["-fsSL", "--retry", "2"]).args(args);
    cmd.env("PATH", full_path()).stdin(Stdio::null());
    hide_console(&mut cmd);
    cmd
}

/// Downloads the current Node.js LTS into ~/.coursmith/node, after checking the file against nodejs.org's checksums.
fn install_node_blocking<R: Runtime>(app: &AppHandle<R>) -> Result<String, String> {
    let suffix = node_archive_suffix()?;
    let base_url = format!("https://nodejs.org/dist/{NODE_LINE}");
    let sums = exec(app, curl(&[&format!("{base_url}/SHASUMS256.txt")]), "Looking up the current Node.js")?;
    let (expected, file) = sums
        .lines()
        .filter_map(|l| l.split_once("  "))
        .find(|(_, name)| name.ends_with(suffix))
        .ok_or("Could not find the Node.js download for this computer.")?;

    let base = base_dir();
    std::fs::create_dir_all(&base).map_err(|e| e.to_string())?;
    let archive = base.join(file);
    exec(
        app,
        curl(&["-o", &archive.to_string_lossy(), &format!("{base_url}/{file}")]),
        &format!("Downloading {file}"),
    )?;
    if sha256_file(&archive)? != expected {
        let _ = std::fs::remove_file(&archive);
        return Err("The Node.js download does not match its checksum: nothing was installed. Try again.".into());
    }

    // tar handles both .tar.gz and (bsdtar, on Windows and macOS) .zip.
    let unpack = base.join("node-unpack");
    let _ = std::fs::remove_dir_all(&unpack);
    std::fs::create_dir_all(&unpack).map_err(|e| e.to_string())?;
    let mut tar = Command::new("tar");
    tar.arg("-xf").arg(&archive).arg("-C").arg(&unpack);
    tar.env("PATH", full_path()).stdin(Stdio::null());
    hide_console(&mut tar);
    exec(app, tar, "Unpacking Node.js")?;

    let inner = std::fs::read_dir(&unpack)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .ok_or("The Node.js archive was empty.")?;
    let _ = std::fs::remove_dir_all(node_dir());
    std::fs::rename(&inner, node_dir()).map_err(|e| format!("Could not move Node.js into place: {e}"))?;
    let _ = std::fs::remove_dir_all(&unpack);
    let _ = std::fs::remove_file(&archive);

    add_to_user_path(app);
    node_version().map(|(v, _)| v).ok_or_else(|| "Node.js was unpacked but does not start.".into())
}

/// Makes the private Node.js and the agents installed with npm available in new terminals.
fn add_to_user_path<R: Runtime>(app: &AppHandle<R>) {
    if cfg!(windows) {
        let script = format!(
            "$dirs = @('{}','{}'); $p = [Environment]::GetEnvironmentVariable('Path','User'); \
             foreach ($d in $dirs) {{ if (-not ($p -split ';' -contains $d)) {{ $p = ($p.TrimEnd(';') + ';' + $d) }} }}; \
             [Environment]::SetEnvironmentVariable('Path', $p, 'User')",
            node_bin_dir().display(),
            npm_bin_dir().display()
        );
        if let Err(e) = exec(app, shell(&script), "Adding Coursmith's tools to your PATH") {
            log(app, e);
        }
        return;
    }
    let marker = "# Added by Coursmith Setup";
    let line = format!("{marker}\nexport PATH=\"$HOME/.coursmith/node/bin:$HOME/.coursmith/npm/bin:$PATH\"\n");
    let h = home();
    let mut targets: Vec<PathBuf> = [".zshrc", ".bashrc", ".bash_profile", ".profile"]
        .iter()
        .map(|f| h.join(f))
        .filter(|p| p.exists())
        .collect();
    if targets.is_empty() {
        targets.push(h.join(if cfg!(target_os = "macos") { ".zshrc" } else { ".profile" }));
    }
    for file in targets {
        let current = std::fs::read_to_string(&file).unwrap_or_default();
        if current.contains(marker) {
            continue;
        }
        let sep = if current.is_empty() || current.ends_with('\n') { "\n" } else { "\n\n" };
        if std::fs::write(&file, format!("{current}{sep}{line}")).is_ok() {
            log(app, format!("Added Coursmith's tools to PATH in {}", file.display()));
        }
    }
}

fn npm() -> Result<PathBuf, String> {
    find_program("npm").ok_or_else(|| "npm was not found. Install Node.js first.".to_string())
}

fn npx() -> Result<PathBuf, String> {
    find_program("npx").ok_or_else(|| "npx was not found. Install Node.js first.".to_string())
}

/// `npx --yes coursmith@latest <args>`
fn coursmith(args: &[&str]) -> Result<Command, String> {
    let mut cmd = command(&npx()?);
    cmd.args(["--yes", CLI]).args(args);
    Ok(cmd)
}

// ---------------------------------------------------------------------------------------------------- the account

/// The Coursmith account saved by `coursmith login`, read from the CLI's credentials file (no network).
fn saved_account() -> Option<String> {
    let dir = if let Some(custom) = std::env::var_os("COURSMITH_CONFIG_DIR") {
        PathBuf::from(custom)
    } else if cfg!(windows) {
        PathBuf::from(std::env::var_os("APPDATA")?).join("coursmith")
    } else if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(xdg).join("coursmith")
    } else {
        home().join(".config").join("coursmith")
    };
    let text = std::fs::read_to_string(dir.join("credentials.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("username")?.as_str().map(str::to_string)
}

/// The last JSON object printed by a `--json` command (npm may print notices before it).
fn last_json(output: &str) -> Option<serde_json::Value> {
    let start = output.rfind("\n{").map(|i| i + 1).or_else(|| output.starts_with('{').then_some(0))?;
    serde_json::from_str(output[start..].trim()).ok()
}

// ------------------------------------------------------------------------------------------------------ commands

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AgentStatus {
    id: &'static str,
    name: &'static str,
    plan: &'static str,
    installed: bool,
    /// False when this system has no automatic installer for it.
    can_install: bool,
    manual_url: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Status {
    os: &'static str,
    node_version: Option<String>,
    node_ok: bool,
    agents: Vec<AgentStatus>,
    account: Option<String>,
    workspace: String,
}

fn can_install(a: &AgentDef) -> bool {
    a.npm.is_some() || if cfg!(windows) { a.windows_script.is_some() } else { a.unix_script.is_some() }
}

async fn blocking<T: Send + 'static>(task: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(task).await.map_err(|e| e.to_string())?
}

/// What is already on this computer.
#[tauri::command]
async fn detect() -> Result<Status, String> {
    blocking(|| {
        let node = node_version();
        Ok(Status {
            os: std::env::consts::OS,
            node_ok: node.as_ref().map(|(_, ok)| *ok).unwrap_or(false),
            node_version: node.map(|(v, _)| v),
            agents: AGENTS
                .iter()
                .map(|a| AgentStatus {
                    id: a.id,
                    name: a.name,
                    plan: a.plan,
                    installed: find_program(a.bin).is_some(),
                    can_install: can_install(a),
                    manual_url: a.manual_url,
                })
                .collect(),
            account: saved_account(),
            workspace: workspace_dir().display().to_string(),
        })
    })
    .await
}

#[tauri::command]
async fn install_node(app: AppHandle) -> Result<String, String> {
    blocking(move || install_node_blocking(&app)).await
}

#[tauri::command]
async fn install_agent(app: AppHandle, id: String) -> Result<(), String> {
    blocking(move || {
        let a = agent(&id)?;
        let script = if cfg!(windows) { a.windows_script } else { a.unix_script };
        if let Some(script) = script {
            exec(&app, shell(script), &format!("Installing {}", a.name))?;
        } else if let Some(package) = a.npm {
            let prefix = npm_prefix();
            std::fs::create_dir_all(&prefix).map_err(|e| e.to_string())?;
            let mut cmd = command(&npm()?);
            cmd.args(["install", "--global", "--prefix"]).arg(&prefix).arg(package);
            exec(&app, cmd, &format!("Installing {}", a.name))?;
            add_to_user_path(&app);
        } else {
            return Err(format!("{} cannot be installed automatically on this system. Install it from {}", a.name, a.manual_url));
        }
        if find_program(a.bin).is_none() {
            return Err(format!(
                "{} was installed but the `{}` command was not found. Close this app, open a new terminal and check `{} --version`.",
                a.name, a.bin, a.bin
            ));
        }
        Ok(())
    })
    .await
}

/// Installs the coursmith-course skill where the chosen agent reads it.
#[tauri::command]
async fn install_skill(app: AppHandle, id: String) -> Result<Vec<String>, String> {
    blocking(move || {
        let a = agent(&id)?;
        let out = exec(&app, coursmith(&["install-skill", "--agent", a.id, "--json"])?, "Installing the Coursmith skill")?;
        let json = last_json(&out).ok_or("The Coursmith CLI did not answer as expected.")?;
        Ok(json
            .get("paths")
            .and_then(|p| p.as_array())
            .map(|p| p.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
            .unwrap_or_default())
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginStart {
    url: String,
    code: String,
}

/// Starts the device login and opens the approval page in the browser.
#[tauri::command]
async fn login_start(app: AppHandle) -> Result<LoginStart, String> {
    blocking(move || {
        let out = exec(&app, coursmith(&["login", "--no-wait", "--no-browser", "--json"])?, "Starting the Coursmith login")?;
        let json = last_json(&out).ok_or("The Coursmith CLI did not answer as expected.")?;
        let field = |k: &str| json.get(k).and_then(|v| v.as_str()).map(str::to_string);
        let start = LoginStart {
            url: field("verificationUri").ok_or("The login did not return a link.")?,
            code: field("userCode").ok_or("The login did not return a code.")?,
        };
        let _ = app.opener().open_url(start.url.clone(), None::<&str>);
        Ok(start)
    })
    .await
}

/// Waits until the login is approved in the browser; returns the username.
#[tauri::command]
async fn login_wait(app: AppHandle) -> Result<String, String> {
    blocking(move || {
        exec(&app, coursmith(&["login", "--resume", "--json"])?, "Waiting for your approval in the browser")?;
        saved_account().ok_or_else(|| "The login finished but no account was saved. Try again.".to_string())
    })
    .await
}

#[tauri::command]
fn open_link(app: AppHandle, url: String) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err("Only https links can be opened.".into());
    }
    app.opener().open_url(url, None::<&str>).map_err(|e| e.to_string())
}

/// Opens a terminal in ~/Coursmith with the agent running.
#[tauri::command]
async fn open_agent(id: String) -> Result<String, String> {
    blocking(move || {
        let a = agent(&id)?;
        let work = workspace_dir();
        std::fs::create_dir_all(&work).map_err(|e| format!("Could not create {}: {e}", work.display()))?;
        std::fs::create_dir_all(base_dir()).map_err(|e| e.to_string())?;
        let path_dirs: Vec<String> = [node_bin_dir(), npm_bin_dir(), home().join(".local").join("bin")]
            .iter()
            .map(|p| p.display().to_string())
            .collect();

        if cfg!(windows) {
            let launcher = base_dir().join(format!("start-{}.cmd", a.id));
            let body = format!(
                "@echo off\r\nset \"PATH={};%PATH%\"\r\ncd /d \"{}\"\r\ntitle Coursmith - {}\r\n{}\r\n",
                path_dirs.join(";"),
                work.display(),
                a.name,
                a.bin
            );
            std::fs::write(&launcher, body).map_err(|e| e.to_string())?;
            let mut cmd = Command::new("cmd");
            cmd.args(["/C", "start", "", "cmd", "/K"]).arg(&launcher);
            cmd.spawn().map_err(|e| format!("Could not open a terminal: {e}"))?;
            return Ok(work.display().to_string());
        }

        let launcher = base_dir().join(format!("start-{}.command", a.id));
        let body = format!(
            "#!/bin/bash\nexport PATH=\"{}:$PATH\"\ncd \"{}\"\necho \"Coursmith workspace: $(pwd)\"\n{}\nexec \"${{SHELL:-/bin/bash}}\"\n",
            path_dirs.join(":"),
            work.display(),
            a.bin
        );
        std::fs::write(&launcher, body).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&launcher, std::fs::Permissions::from_mode(0o755));
        }

        if cfg!(target_os = "macos") {
            // Finder opens .command files in Terminal.
            Command::new("open").arg(&launcher).spawn().map_err(|e| format!("Could not open Terminal: {e}"))?;
            return Ok(work.display().to_string());
        }

        // Linux: the first terminal emulator found. Most accept `-e <program>`.
        let terminals: [(&str, &[&str]); 11] = [
            ("x-terminal-emulator", &["-e"]),
            ("ghostty", &["-e"]),
            ("kitty", &[]),
            ("alacritty", &["-e"]),
            ("foot", &[]),
            ("wezterm", &["start", "--"]),
            ("gnome-terminal", &["--"]),
            ("kgx", &["-e"]),
            ("konsole", &["-e"]),
            ("xfce4-terminal", &["-x"]),
            ("xterm", &["-e"]),
        ];
        for (term, args) in terminals {
            if let Some(path) = find_program(term) {
                if Command::new(path).args(args).arg(&launcher).spawn().is_ok() {
                    return Ok(work.display().to_string());
                }
            }
        }
        Err(format!("No terminal app was found. Open a terminal yourself and run: cd \"{}\" && {}", work.display(), a.bin))
    })
    .await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            detect,
            install_node,
            install_agent,
            install_skill,
            login_start,
            login_wait,
            open_link,
            open_agent
        ])
        .run(tauri::generate_context!())
        .expect("error while running Coursmith Setup");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_last_json_object_after_npm_notices() {
        let out = "npm warn something\n{\n  \"status\": \"pending\",\n  \"userCode\": \"ABCD-1234\"\n}\n";
        assert_eq!(last_json(out).unwrap()["userCode"], "ABCD-1234");
        assert_eq!(last_json("{\"a\":1}").unwrap()["a"], 1);
        assert!(last_json("no json here").is_none());
    }

    #[test]
    fn every_agent_has_a_unique_id_and_some_way_in() {
        let mut ids: Vec<_> = AGENTS.iter().map(|a| a.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), AGENTS.len());
        for a in AGENTS {
            assert!(a.manual_url.starts_with("https://"), "{}", a.id);
        }
    }

    #[test]
    fn knows_the_node_download_for_this_machine() {
        assert!(node_archive_suffix().is_ok());
    }

    /// Downloads Node.js and installs the skill into a throw-away home folder. Needs the network:
    /// `cargo test -- --ignored --test-threads=1`.
    #[test]
    #[ignore]
    fn installs_node_and_the_skill_in_an_empty_home() {
        let fake_home = std::env::temp_dir().join(format!("coursmith-setup-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&fake_home);
        std::fs::create_dir_all(&fake_home).unwrap();
        std::env::set_var("HOME", &fake_home);
        std::env::set_var("USERPROFILE", &fake_home);
        // Hide any system Node.js, so the private one is the only one found.
        std::env::set_var("PATH", "/usr/bin:/bin");
        std::env::remove_var("XDG_CONFIG_HOME");

        let app = tauri::test::mock_app();
        let handle = app.handle().clone();
        let version = install_node_blocking(&handle).expect("node installs");
        assert!(version.starts_with("v22."), "{version}");
        assert!(node_bin_dir().join(if cfg!(windows) { "node.exe" } else { "node" }).is_file());
        let profile = std::fs::read_to_string(fake_home.join(".profile")).unwrap_or_default();
        assert!(cfg!(windows) || profile.contains(".coursmith/node/bin"), "PATH line written");

        let out = exec(&handle, coursmith(&["install-skill", "--agent", "antigravity", "--json"]).unwrap(), "skill").expect("skill installs");
        let json = last_json(&out).expect("json");
        assert_eq!(json["status"], "installed");
        assert!(fake_home.join(".gemini/antigravity-cli/skills/coursmith-course/SKILL.md").is_file());

        let out = exec(&handle, coursmith(&["login", "--no-wait", "--no-browser", "--json"]).unwrap(), "login").expect("login starts");
        let json = last_json(&out).expect("json");
        assert!(json["verificationUri"].as_str().unwrap().starts_with("https://"));
        assert!(saved_account().is_none());

        let _ = std::fs::remove_dir_all(&fake_home);
    }
}
