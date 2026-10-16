// Lab console UI logic. Talks to the Rust backend over REST + WebSocket.
// No framework/CDN so it runs on an isolated LAN.

const api = {
  async get(p) { return (await fetch(p)).json(); },
  async post(p, body) {
    const r = await fetch(p, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body ?? {}),
    });
    return r.json();
  },
};

const state = { clients: new Map() };

const $ = (id) => document.getElementById(id);

function toast(msg, ms = 2600) {
  const t = $("toast");
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toast._t);
  toast._t = setTimeout(() => (t.hidden = true), ms);
}

// Disable a button and show progress while `fn` runs: LAN operations can
// take seconds, and an unchanged button looks like the click was lost.
async function busy(btn, label, fn) {
  if (btn.disabled) return;
  const old = btn.innerHTML;
  btn.disabled = true;
  btn.innerHTML = `<span class="spinner"></span>${label}`;
  try {
    return await fn();
  } catch {
    toast("请求失败：无法连接教师端服务");
  } finally {
    btn.disabled = false;
    btn.innerHTML = old;
  }
}

// One toast for a bulk action: totals, then each failing (or partly
// failing) terminal with its reason.
function summarize(title, res) {
  const results = res.results || [];
  const name = (id) => state.clients.get(id)?.hostname || id;
  const reason = (d) => (d && (d.error || d.message || d.note)) || "未知原因";
  const failed = results.filter((r) => !r.ok);
  const partial = results.filter((r) => r.ok && /部分失败/.test(r.detail?.note || ""));
  const lines = [`${title}：${results.length - failed.length}/${results.length} 台成功`];
  for (const r of failed) lines.push(`✗ ${name(r.id)}：${reason(r.detail)}`);
  for (const r of partial) lines.push(`! ${name(r.id)}：${r.detail.note.replace(/^.*?部分失败：/, "")}`);
  toast(lines.join("\n"), failed.length || partial.length ? 9000 : 2600);
}

// Display text and style for ClientStatus.uwf_state (enabling takes reboots).
function uwfLabel(st) {
  const s = st.uwf_state;
  if (!s) return st.uwf_enabled === true ? ["UWF:开", "pill-ok"] : st.uwf_enabled === false ? ["UWF:关", ""] : ["UWF:—", ""];
  if (s.startsWith("error:")) return [`UWF:出错`, "pill-bad", s.slice(6)];
  return {
    on: ["UWF:已开启", "pill-ok"],
    on_next_boot: ["UWF:重启后生效", "pill-warn"],
    off_next_boot: ["UWF:重启后关闭", "pill-warn"],
    off: ["UWF:关", ""],
    installing: ["UWF:安装中…", "pill-warn"],
    install_pending_reboot: ["UWF:已安装，需重启", "pill-warn"],
    not_installed: ["UWF:未安装", ""],
  }[s] || [`UWF:${s}`, ""];
}

function usbLabel(v) {
  return { allow: "U盘:允许", read_only: "U盘:只读", deny: "U盘:禁用" }[v] || "U盘:—";
}

// Text from terminals goes into innerHTML; never let it become markup.
const esc = (v) =>
  String(v ?? "").replace(/[&<>"']/g, (ch) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[ch]);

// Every problem a terminal reports, in full: UWF errors, policy apply
// failures, and why its screen could not be captured.
function issuesOf(c) {
  const st = c.status || {};
  const out = [];
  if (st.uwf_state?.startsWith("error:")) out.push(`UWF：${st.uwf_state.slice(6)}`);
  for (const e of st.errors || []) out.push(e);
  if (c.thumbnail_error) out.push(`画面：${c.thumbnail_error}`);
  return out;
}

function render() {
  const grid = $("grid");
  const list = [...state.clients.values()].sort((a, b) => a.hostname.localeCompare(b.hostname));
  $("stat-total").textContent = list.length;
  $("stat-online").textContent = list.filter((c) => c.online).length;
  $("stat-offline").textContent = list.filter((c) => !c.online).length;
  $("empty").hidden = list.length > 0;

  grid.innerHTML = "";
  for (const c of list) {
    const st = c.status || {};
    const card = document.createElement("div");
    card.className = "card" + (c.online ? "" : " offline");

    const thumb = c.thumbnail
      ? `<img class="thumb" alt="screen" src="data:image/jpeg;base64,${c.thumbnail.jpeg_base64}" />`
      : `<div class="thumb thumb-empty">无画面${c.thumbnail_error ? `<small>${esc(c.thumbnail_error)}</small>` : ""}</div>`;

    const [uwfText, uwfClass, uwfTip] = uwfLabel(st);
    const uwf = `<span class="pill ${uwfClass}" title="${esc(uwfTip)}">${esc(uwfText)}</span>`;
    const user = st.logged_in_user ? `用户:${esc(st.logged_in_user)}` : "未登录";
    const lock = st.session_locked ? `<span class="pill">口令保护</span>` : "";
    // Without a screenshot the capture error is already shown in its place.
    const issues = issuesOf(c).filter((i) => c.thumbnail || !i.startsWith("画面："));
    const issueList = issues.length
      ? `<ul class="card-issues">${issues.map((i) => `<li>${esc(i)}</li>`).join("")}</ul>`
      : "";

    card.innerHTML = `
      <a class="thumb-wrap" data-view="${c.machine_id}" title="点击查看全屏画面与详细状态">${thumb}</a>
      <div class="card-body">
        <div class="card-title">
          <h3><span class="dot ${c.online ? "on" : "off"}"></span> ${esc(c.hostname)}</h3>
          <span class="muted" style="font-size:12px">${esc(c.addr)}</span>
        </div>
        <div class="meta">
          <span class="pill">${user}</span>
          ${uwf}
          <span class="pill">rev ${esc(st.policy_revision ?? 0)}</span>
          ${lock}
        </div>
        ${issueList}
      </div>
      <div class="card-actions">
        <button class="btn btn-outline btn-sm" data-act="view" data-id="${c.machine_id}">查看</button>
        <button class="btn btn-outline btn-sm" data-act="launch" data-id="${c.machine_id}">唤起</button>
        <button class="btn btn-danger btn-sm" data-act="reboot" data-id="${c.machine_id}">重启还原</button>
      </div>`;
    grid.appendChild(card);
  }
  renderViewer();
}

function upsert(client) {
  state.clients.set(client.machine_id, client);
}

// --- WebSocket live updates ---
function connectWs() {
  const proto = location.protocol === "https:" ? "wss" : "ws";
  const ws = new WebSocket(`${proto}://${location.host}/ws`);
  const conn = $("conn");
  ws.onopen = () => { conn.textContent = "已连接"; conn.className = "badge badge-ok"; };
  ws.onclose = () => {
    conn.textContent = "已断开，重连中…"; conn.className = "badge badge-bad";
    setTimeout(connectWs, 1500);
  };
  ws.onmessage = (ev) => {
    const msg = JSON.parse(ev.data);
    if (msg.kind === "roster") {
      state.clients = new Map(msg.clients.map((c) => [c.machine_id, c]));
      render();
    } else if (msg.kind === "thumbnail") {
      const c = state.clients.get(msg.machine_id);
      if (c) { c.thumbnail = msg.thumbnail; render(); }
    }
  };
}

// --- Actions ---
document.addEventListener("click", async (e) => {
  const shot = e.target.closest("[data-view]");
  if (shot) return openViewer(shot.dataset.view);
  const btn = e.target.closest("button[data-act]");
  if (!btn) return;
  const id = btn.dataset.id;
  if (btn.dataset.act === "view") return openViewer(id);
  const done = (res, okMsg) =>
    toast(res.ok && res.report?.type !== "error" ? okMsg : `失败：${res.error || res.report?.message || "未知原因"}`);
  if (btn.dataset.act === "launch") {
    await busy(btn, "唤起中…", async () => done(await api.post(`/api/clients/${id}/launch`), "已发送唤起指令"));
  } else if (btn.dataset.act === "reboot") {
    if (!confirm("确定重启该终端并恢复到还原点？")) return;
    await busy(btn, "发送中…", async () =>
      done(await api.post(`/api/clients/${id}/reboot`, { restore: true, delay_secs: 5 }), "已发送重启还原指令"));
  }
});


// Run a per-client endpoint for every terminal and collect bulk-style results.
async function forAll(path, body) {
  const ids = [...state.clients.keys()];
  const results = await Promise.all(ids.map(async (id) => {
    const r = await api.post(`/api/clients/${id}/${path}`, body).catch(() => ({ ok: false, error: "请求失败" }));
    const ok = r.ok && r.report?.type !== "error";
    return { id, ok, detail: ok ? r.report : { error: r.error || r.report?.message } };
  }));
  return { results };
}

$("btn-launch-all").onclick = (e) =>
  busy(e.currentTarget, "唤起中…", async () => summarize("唤起客户端", await forAll("launch")));

$("btn-reboot-all").onclick = (e) => {
  if (!confirm("确定重启全部终端并恢复到还原点？")) return;
  return busy(e.currentTarget, "发送中…", async () =>
    summarize("重启还原", await forAll("reboot", { restore: true, delay_secs: 5 })));
};

// --- Policy drawer ---
const drawer = $("drawer");
const openDrawer = () => (drawer.hidden = false);
const closeDrawer = () => (drawer.hidden = true);
$("btn-policy").onclick = openDrawer;
$("drawer-close").onclick = closeDrawer;
$("drawer-cancel").onclick = closeDrawer;
drawer.onclick = (e) => { if (e.target === drawer) closeDrawer(); };

// --- Drawer settings persistence (survive console restart) ---
function collectSettings() {
  return {
    lock_wallpaper: $("p-wallpaper").checked,
    wallpaper_path: $("p-wallpaper-path").value,
    usb_storage: $("p-usb").value,
    uwf: $("p-uwf").value,
    uwf_scope: $("p-uwf-scope").value,
    disable_task_manager: $("p-taskmgr").checked,
    block_internet: $("p-block-internet").checked,
    default_wallpaper: $("d-wallpaper").value,
  };
}

function applySettings(s) {
  if (!s || typeof s !== "object") return;
  if (typeof s.lock_wallpaper === "boolean") $("p-wallpaper").checked = s.lock_wallpaper;
  if (typeof s.wallpaper_path === "string") $("p-wallpaper-path").value = s.wallpaper_path;
  if (s.usb_storage) $("p-usb").value = s.usb_storage;
  if (s.uwf) $("p-uwf").value = s.uwf;
  if (s.uwf_scope) $("p-uwf-scope").value = s.uwf_scope;
  if (typeof s.disable_task_manager === "boolean") $("p-taskmgr").checked = s.disable_task_manager;
  if (typeof s.block_internet === "boolean") $("p-block-internet").checked = s.block_internet;
  if (typeof s.default_wallpaper === "string") $("d-wallpaper").value = s.default_wallpaper;
}

api.get("/api/settings").then(applySettings).catch(() => {});

$("p-wallpaper-pick").onclick = async () => {
  try {
    const { path } = await api.post("/api/pick-wallpaper");
    if (path) $("p-wallpaper-path").value = path;
  } catch { toast("无法打开文件选择框"); }
};


$("btn-update-client").onclick = async (e) => {
  const btn = e.currentTarget;
  let src;
  try {
    const r = await api.post("/api/pick-exe");
    src = r.path;
  } catch { toast("无法打开文件选择框"); return; }
  if (!src) return; // cancelled
  const target = prompt("学生机安装路径（留空＝与学生端服务同目录）：", "") || null;
  const n = state.clients.size;
  if (!confirm(`将所选客户端下发并无感更新到 ${n} 台终端，更新后自动运行？\n\n源文件：${src}${target ? "\n目标：" + target : ""}`)) return;
  await busy(btn, "下发中…", async () => {
    const res = await api.post("/api/update-client", { source_path: src, target_path: target, run_after: true });
    summarize(`客户端更新（${Math.round((res.size || 0) / 1024)} KB）`, res);
  });
};

// --- One-click restore defaults dialog ---
const defaultsModal = $("defaults-modal");
const closeDefaults = () => (defaultsModal.hidden = true);
$("btn-defaults-all").onclick = () => {
  $("defaults-count").textContent = state.clients.size;
  defaultsModal.hidden = false;
};
$("defaults-close").onclick = closeDefaults;
$("defaults-cancel").onclick = closeDefaults;
defaultsModal.onclick = (e) => { if (e.target === defaultsModal) closeDefaults(); };

$("d-wallpaper-pick").onclick = async () => {
  try {
    const { path } = await api.post("/api/pick-wallpaper");
    if (path) $("d-wallpaper").value = path;
  } catch { toast("无法打开文件选择框"); }
};

$("defaults-apply").onclick = (e) => {
  const wallpaper = $("d-wallpaper").value.trim();
  // Remember the chosen default wallpaper for next time.
  api.post("/api/settings", collectSettings()).catch(() => {});
  return busy(e.currentTarget, "恢复中…", async () => {
    const res = await api.post("/api/apply-defaults", { wallpaper_path: wallpaper || null });
    closeDefaults();
    summarize(`一键恢复默认${wallpaper ? "（含默认壁纸）" : ""}`, res);
  });
};

$("drawer-apply").onclick = (e) => busy(e.currentTarget, "下发中…", async () => {
  const uwfSel = $("p-uwf").value;
  const policy = {
    lock_wallpaper: $("p-wallpaper").checked,
    wallpaper_path: $("p-wallpaper-path").value.trim() || null,
    usb_storage: $("p-usb").value,
    uwf_enabled: uwfSel === "unmanaged" ? null : uwfSel === "true",
    uwf_scope: $("p-uwf-scope").value,
    disable_task_manager: $("p-taskmgr").checked,
    block_internet: $("p-block-internet").checked,
    keep_service_alive: true,
    revision: Date.now(), // monotonic-enough per operator session
  };

  const pass = $("p-pass").value;
  if (pass) {
    const { hash } = await api.post("/api/hash", { passphrase: pass });
    policy.config_passphrase_hash = hash;
  }

  // Persist the drawer choices (not the plaintext password) for next time.
  api.post("/api/settings", collectSettings()).catch(() => {});

  const res = await api.post("/api/policy", policy);
  closeDrawer();
  summarize("策略下发", res);
});

// --- Full-screen terminal view ---
const viewer = { id: null, timer: null, loading: false };

function openViewer(id) {
  viewer.id = id;
  $("v-img").hidden = true;
  $("v-noimg").hidden = false;
  $("v-noimg").textContent = "正在获取画面…";
  $("viewer").hidden = false;
  renderViewer();
  refreshScreen();
  clearInterval(viewer.timer);
  viewer.timer = setInterval(refreshScreen, 2000);
}

function closeViewer() {
  if (document.fullscreenElement) document.exitFullscreen().catch(() => {});
  $("viewer").hidden = true;
  viewer.id = null;
  clearInterval(viewer.timer);
}

// A larger capture than the grid thumbnail, refreshed while the view is open.
async function refreshScreen() {
  const id = viewer.id;
  if (!id || viewer.loading) return;
  viewer.loading = true;
  try {
    const r = await api.get(`/api/clients/${id}/screen?max=1600`);
    if (viewer.id !== id) return;
    if (r.ok && r.report?.type === "thumbnail") {
      $("v-img").src = `data:image/jpeg;base64,${r.report.jpeg_base64}`;
      $("v-img").hidden = false;
      $("v-noimg").hidden = true;
      $("v-updated").textContent = `更新于 ${new Date().toLocaleTimeString()}`;
    } else {
      $("v-img").hidden = true;
      $("v-noimg").hidden = false;
      $("v-noimg").textContent = `无法获取画面：${r.error || r.report?.message || "未知原因"}`;
    }
  } catch {
    $("v-noimg").textContent = "无法连接教师端服务";
  } finally {
    viewer.loading = false;
  }
}

function renderViewer() {
  const c = viewer.id && state.clients.get(viewer.id);
  if (!c) return;
  const st = c.status || {};
  $("v-host").textContent = c.hostname;
  $("v-addr").textContent = c.addr;
  const rows = [
    ["状态", c.online ? "在线" : "离线"],
    ["登录用户", st.logged_in_user || "—"],
    ["地址", c.addr],
    ["客户端版本", st.agent_version || "—"],
    ["UWF", uwfLabel(st)[0].replace(/^UWF:/, "")],
    ["策略版本", st.policy_revision ?? 0],
    ["口令保护", st.session_locked ? "是" : "否"],
    ["最后在线", c.last_seen ? new Date(c.last_seen).toLocaleString() : "—"],
  ];
  $("v-info").innerHTML = rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${esc(v)}</dd>`).join("");
  const issues = issuesOf(c);
  $("v-issues").innerHTML = issues.length
    ? issues.map((i) => `<li>${esc(i)}</li>`).join("")
    : `<li class="ok">没有错误</li>`;
}

$("v-close").onclick = closeViewer;
$("v-full").onclick = () =>
  document.fullscreenElement ? document.exitFullscreen() : $("viewer").requestFullscreen().catch(() => {});
document.addEventListener("fullscreenchange", () => {
  $("v-full").textContent = document.fullscreenElement ? "退出全屏" : "全屏";
});
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape" && viewer.id && !document.fullscreenElement) closeViewer();
});

// boot
connectWs();
api.get("/api/roster").then((list) => { list.forEach(upsert); render(); }).catch(() => {});
