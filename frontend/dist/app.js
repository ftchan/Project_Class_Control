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

function toast(msg) {
  const t = $("toast");
  t.textContent = msg;
  t.hidden = false;
  clearTimeout(toast._t);
  toast._t = setTimeout(() => (t.hidden = true), 2600);
}

function usbLabel(v) {
  return { allow: "U盘:允许", read_only: "U盘:只读", deny: "U盘:禁用" }[v] || "U盘:—";
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
      : `<div class="thumb thumb-empty">无画面</div>`;

    const uwf = st.uwf_enabled === true ? "UWF:开" : st.uwf_enabled === false ? "UWF:关" : "UWF:—";
    const user = st.logged_in_user ? `用户:${st.logged_in_user}` : "未登录";
    const lock = st.session_locked ? `<span class="pill">口令保护</span>` : "";

    card.innerHTML = `
      ${thumb}
      <div class="card-body">
        <div class="card-title">
          <h3><span class="dot ${c.online ? "on" : "off"}"></span> ${c.hostname}</h3>
          <span class="muted" style="font-size:12px">${c.addr}</span>
        </div>
        <div class="meta">
          <span class="pill">${user}</span>
          <span class="pill">${uwf}</span>
          <span class="pill">rev ${st.policy_revision ?? 0}</span>
          ${lock}
        </div>
      </div>
      <div class="card-actions">
        <button class="btn btn-outline btn-sm" data-act="launch" data-id="${c.machine_id}">唤起</button>
        <button class="btn btn-danger btn-sm" data-act="reboot" data-id="${c.machine_id}">重启还原</button>
      </div>`;
    grid.appendChild(card);
  }
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
  const btn = e.target.closest("button[data-act]");
  if (!btn) return;
  const id = btn.dataset.id;
  if (btn.dataset.act === "launch") {
    await api.post(`/api/clients/${id}/launch`);
    toast("已发送唤起指令");
  } else if (btn.dataset.act === "reboot") {
    if (!confirm("确定重启该终端并恢复到还原点？")) return;
    await api.post(`/api/clients/${id}/reboot`, { restore: true, delay_secs: 5 });
    toast("已发送重启还原指令");
  }
});

$("btn-launch-all").onclick = async () => {
  const ids = [...state.clients.keys()];
  await Promise.all(ids.map((id) => api.post(`/api/clients/${id}/launch`)));
  toast(`已唤起 ${ids.length} 台终端`);
};

$("btn-reboot-all").onclick = async () => {
  if (!confirm("确定重启全部终端并恢复到还原点？")) return;
  const ids = [...state.clients.keys()];
  await Promise.all(ids.map((id) => api.post(`/api/clients/${id}/reboot`, { restore: true, delay_secs: 5 })));
  toast(`已向 ${ids.length} 台终端下发重启还原`);
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
    default_wallpaper: $("p-default-wallpaper").value,
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
  if (typeof s.default_wallpaper === "string") $("p-default-wallpaper").value = s.default_wallpaper;
}

api.get("/api/settings").then(applySettings).catch(() => {});

$("p-wallpaper-pick").onclick = async () => {
  try {
    const { path } = await api.post("/api/pick-wallpaper");
    if (path) $("p-wallpaper-path").value = path;
  } catch { toast("无法打开文件选择框"); }
};

$("p-default-wallpaper-pick").onclick = async () => {
  try {
    const { path } = await api.post("/api/pick-wallpaper");
    if (path) { $("p-default-wallpaper").value = path; api.post("/api/settings", collectSettings()).catch(() => {}); }
  } catch { toast("无法打开文件选择框"); }
};

$("btn-defaults-all").onclick = async () => {
  const n = state.clients.size;
  if (!confirm(`将 ${n} 台终端的鼠标样式、桌面图标大小恢复为默认${$("p-default-wallpaper").value ? "，并统一设置默认壁纸" : ""}？`)) return;
  const res = await api.post("/api/apply-defaults", { wallpaper_path: $("p-default-wallpaper").value || null });
  const ok = (res.results || []).filter((r) => r.ok).length;
  toast(`一键恢复默认：${ok}/${(res.results || []).length} 台已下发`);
};

$("drawer-apply").onclick = async () => {
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
  const ok = (res.results || []).filter((r) => r.ok).length;
  const total = (res.results || []).length;
  closeDrawer();
  toast(`策略已下发：${ok}/${total} 台成功`);
};

// boot
connectWs();
api.get("/api/roster").then((list) => { list.forEach(upsert); render(); }).catch(() => {});
