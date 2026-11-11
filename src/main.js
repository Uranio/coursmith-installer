// Coursmith Setup wizard: choose an agent → install → connect the account → ready.
const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const SITE = "https://coursmith.onrender.com";

/** The three common subscriptions, each with the agent that works with it. */
const PLANS = [
  { agent: "claude", title: "Claude", sub: "Pro or Max subscription · installs Claude Code" },
  { agent: "codex", title: "ChatGPT", sub: "Plus, Pro or Business subscription · installs Codex" },
  { agent: "antigravity", title: "Gemini", sub: "Google AI Pro, Ultra or a free Google account · installs Antigravity CLI" },
];

const $ = (id) => document.getElementById(id);
const screens = ["choose", "install", "connect", "done"];

let status = null;
let chosen = null;
let login = null;

function show(name) {
  for (const s of screens) $(s).hidden = s !== name;
  const at = screens.indexOf(name);
  document.querySelectorAll(".steps li").forEach((li, i) => {
    li.classList.toggle("current", i === at);
    li.classList.toggle("past", i < at);
    if (i === at) li.setAttribute("aria-current", "step");
    else li.removeAttribute("aria-current");
  });
  document.querySelector("main").scrollTop = 0;
}

const agentInfo = (id) => status.agents.find((a) => a.id === id);

function card({ agent, title, sub }) {
  const info = agentInfo(agent);
  const button = document.createElement("button");
  button.className = "card";
  button.type = "button";
  const text = document.createElement("span");
  const t = document.createElement("span");
  t.className = "title";
  t.textContent = title;
  const s = document.createElement("span");
  s.className = "sub";
  s.textContent = sub;
  text.append(t, s);
  button.append(text);
  const badge = document.createElement("span");
  badge.className = `badge${info.installed ? " ok" : ""}`;
  badge.textContent = info.installed ? "Installed" : info.canInstall ? "Will be installed" : "Manual install";
  button.append(badge);
  button.addEventListener("click", () => start(agent));
  return button;
}

async function refresh() {
  status = await invoke("detect");
  $("plans").replaceChildren(...PLANS.map(card));
  const others = status.agents.filter((a) => !PLANS.some((p) => p.agent === a.id));
  $("others").replaceChildren(...others.map((a) => card({ agent: a.id, title: a.name, sub: a.plan })));
  const found = status.agents.filter((a) => a.installed).map((a) => a.name);
  const parts = [];
  parts.push(found.length ? `Found on this computer: ${found.join(", ")}.` : "No AI agent was found on this computer yet.");
  if (status.account) parts.push(`Coursmith account connected: @${status.account}.`);
  $("detect-note").textContent = parts.join(" ");
}

// ------------------------------------------------------------------------------------------------ install

function task(id, label) {
  const li = document.createElement("li");
  li.id = `task-${id}`;
  const icon = document.createElement("span");
  icon.className = "icon";
  icon.setAttribute("aria-hidden", "true");
  const name = document.createElement("span");
  name.className = "label";
  name.textContent = label;
  const detail = document.createElement("span");
  detail.className = "detail";
  li.append(icon, name, detail);
  return li;
}

function setTask(id, state, detail) {
  const li = $(`task-${id}`);
  li.className = state;
  li.querySelector(".icon").textContent = state === "ok" ? "✓" : state === "failed" ? "!" : "";
  if (detail !== undefined) li.querySelector(".detail").textContent = detail;
}

async function start(agentId) {
  chosen = agentInfo(agentId);
  $("install-title").textContent = `Setting up ${chosen.name}`;
  $("tasks").replaceChildren(
    task("node", "Node.js (runs the Coursmith tools)"),
    task("agent", chosen.name),
    task("skill", "Coursmith skill"),
  );
  $("log").textContent = "";
  show("install");
  await runInstall();
}

async function runInstall() {
  $("install-error").hidden = true;
  $("install-retry").hidden = true;
  $("install-back").disabled = true;
  let current = "node";
  try {
    status = await invoke("detect");
    chosen = agentInfo(chosen.id);

    if (status.nodeOk) {
      setTask("node", "ok", `${status.nodeVersion} found`);
    } else {
      setTask("node", "running", status.nodeVersion ? `${status.nodeVersion} is too old: installing a newer one` : "downloading…");
      const version = await invoke("install_node");
      setTask("node", "ok", `${version} installed`);
    }

    current = "agent";
    if (chosen.installed) {
      setTask("agent", "ok", "already installed");
    } else if (!chosen.canInstall) {
      throw new Error(`${chosen.name} cannot be installed automatically on this system. Install it from ${chosen.manualUrl}, then press Try again.`);
    } else {
      setTask("agent", "running", "installing… this can take a few minutes");
      await invoke("install_agent", { id: chosen.id });
      setTask("agent", "ok", "installed");
    }

    current = "skill";
    setTask("skill", "running", "installing…");
    await invoke("install_skill", { id: chosen.id });
    setTask("skill", "ok", "installed");

    status = await invoke("detect");
    if (status.account) showDone();
    else await connect();
  } catch (err) {
    setTask(current, "failed", "failed");
    $("install-error").textContent = String(err?.message ?? err);
    $("install-error").hidden = false;
    $("install-retry").hidden = false;
  } finally {
    $("install-back").disabled = false;
  }
}

// ------------------------------------------------------------------------------------------------ connect

async function connect() {
  show("connect");
  $("connect-error").hidden = true;
  $("connect-retry").hidden = true;
  $("connect-status").textContent = "Starting…";
  $("code").textContent = "····-····";
  try {
    login = await invoke("login_start");
    $("code").textContent = login.code;
    $("connect-status").textContent = "Waiting for your approval in the browser…";
    await invoke("login_wait");
    status = await invoke("detect");
    showDone();
  } catch (err) {
    $("connect-status").textContent = "";
    $("connect-error").textContent = String(err?.message ?? err);
    $("connect-error").hidden = false;
    $("connect-retry").hidden = false;
  }
}

// --------------------------------------------------------------------------------------------------- done

function showDone() {
  $("done-lead").textContent = `${chosen.name} has the Coursmith skill, and your account @${status.account} is connected.`;
  $("workspace").textContent = status.workspace;
  $("open-agent").textContent = `Open ${chosen.name}`;
  $("open-note").textContent = "";
  show("done");
}

$("install-back").addEventListener("click", async () => {
  await refresh();
  show("choose");
});
$("install-retry").addEventListener("click", runInstall);
$("connect-retry").addEventListener("click", connect);
$("connect-open").addEventListener("click", () => login && invoke("open_link", { url: login.url }));
$("open-agent").addEventListener("click", async () => {
  try {
    await invoke("open_agent", { id: chosen.id });
    $("open-note").textContent = "A terminal window opened with your agent.";
  } catch (err) {
    $("open-note").textContent = String(err?.message ?? err);
  }
});
$("copy").addEventListener("click", async () => {
  const text = $("prompt").textContent;
  try {
    await navigator.clipboard.writeText(text);
    $("copy").textContent = "Copied";
  } catch {
    // Clipboard blocked: select the text so Ctrl/Cmd+C works.
    const range = document.createRange();
    range.selectNodeContents($("prompt"));
    getSelection().removeAllRanges();
    getSelection().addRange(range);
    $("copy").textContent = "Selected: press Ctrl+C";
  }
  setTimeout(() => ($("copy").textContent = "Copy"), 2500);
});
$("open-dashboard").addEventListener("click", () => invoke("open_link", { url: `${SITE}/dashboard` }));
$("restart").addEventListener("click", async () => {
  await refresh();
  show("choose");
});

listen("log", (event) => {
  const log = $("log");
  log.textContent += `${event.payload}\n`;
  if (log.textContent.length > 60000) log.textContent = log.textContent.slice(-40000);
  log.scrollTop = log.scrollHeight;
});

refresh()
  .then(() => show("choose"))
  .catch((err) => {
    $("detect-note").textContent = `Could not inspect this computer: ${err}`;
  });
