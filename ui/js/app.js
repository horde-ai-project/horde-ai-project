const STORAGE_KEY = "horde.operator.apiKey";
const HISTORY_KEY = "horde.operator.jobHistory";
const THEME_KEY = "horde.operator.theme";
const THEME_SYNC_KEY = "horde.operator.themeOsSync";
const THEME_LIGHT_KEY = "horde.operator.themeLight";
const THEME_DARK_KEY = "horde.operator.themeDark";
const TEMPLATES_KEY = "horde.operator.jobTemplates";
const CHECKLIST_DISMISS_KEY = "horde.operator.checklistDismissed";
const CHECKLIST_STATE_KEY = "horde.operator.checklistState";
const PREVIEW_MAX_CHARS = 48_000;
const SLOW_PEER_RTT_MS = 250;
const BASE_POLL_MS = 5000;

const THEMES = [
  { id: "slate-teal", label: "Slate teal (default)", scheme: "light" },
  { id: "coffee", label: "Coffee", scheme: "light" },
  { id: "business-blue", label: "Business blue", scheme: "light" },
  { id: "forest", label: "Forest", scheme: "light" },
  { id: "graphite", label: "Graphite", scheme: "light" },
  { id: "sandstone", label: "Sandstone", scheme: "light" },
  { id: "ocean", label: "Ocean", scheme: "light" },
  { id: "ink", label: "Ink", scheme: "light" },
  { id: "rosewood", label: "Rosewood", scheme: "light" },
  { id: "amber-field", label: "Amber field", scheme: "light" },
  { id: "night-slate", label: "Night slate (dark)", scheme: "dark" },
  { id: "charcoal", label: "Charcoal (dark)", scheme: "dark" },
];

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

/** @type {{ online: boolean, throttledUntil: number, pollBackoffMs: number, slowPeers: string[], lastInvite: object|null, previewBlob: string|null, pollTimer: number|null, inviteTimer: number|null }} */
const runtime = {
  online: true,
  throttledUntil: 0,
  pollBackoffMs: BASE_POLL_MS,
  slowPeers: [],
  lastInvite: null,
  previewBlob: null,
  pollTimer: null,
  inviteTimer: null,
};

function apiBase() {
  return "";
}

function getKey() {
  return ($("#api-key")?.value || "").trim() || localStorage.getItem(STORAGE_KEY) || "";
}

function setConn(state, label) {
  const pill = $("#conn-pill");
  if (!pill) return;
  pill.dataset.state = state;
  pill.textContent = label;
}

function setApiOnline(online) {
  runtime.online = online;
  document.body.classList.toggle("is-api-down", !online);
  const banner = $("#offline-banner");
  if (banner) banner.hidden = online;
  setConn(online ? "ok" : "bad", online ? "online" : "offline");
}

function setThrottle(message, untilMs) {
  if (untilMs) runtime.throttledUntil = Math.max(runtime.throttledUntil, untilMs);
  const banner = $("#throttle-banner");
  const detail = $("#throttle-detail");
  const active =
    Date.now() < runtime.throttledUntil ||
    runtime.pollBackoffMs > BASE_POLL_MS ||
    runtime.slowPeers.length > 0;
  if (banner) banner.hidden = !active;
  if (detail && message) detail.textContent = message;
  else if (detail && active) {
    const parts = [];
    if (Date.now() < runtime.throttledUntil) {
      parts.push(`API rate-limit / backoff until ${new Date(runtime.throttledUntil).toLocaleTimeString()}`);
    }
    if (runtime.pollBackoffMs > BASE_POLL_MS) {
      parts.push(`UI poll interval ${Math.round(runtime.pollBackoffMs / 1000)}s`);
    }
    if (runtime.slowPeers.length) {
      parts.push(`Slow peer RTT ≥ ${SLOW_PEER_RTT_MS}ms: ${runtime.slowPeers.slice(0, 3).join(", ")}`);
    }
    detail.textContent = parts.join(" · ") || "Throttling active";
  }
}

function clearThrottleIfIdle() {
  if (
    Date.now() >= runtime.throttledUntil &&
    runtime.pollBackoffMs <= BASE_POLL_MS &&
    !runtime.slowPeers.length
  ) {
    const banner = $("#throttle-banner");
    if (banner) banner.hidden = true;
  } else {
    setThrottle(null);
  }
}

async function api(path, { method = "GET", body, auth = true } = {}) {
  const headers = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (auth) {
    const key = getKey();
    if (!key) throw new Error("Set and save an API key first (from the horde startup log).");
    headers.Authorization = `Bearer ${key}`;
  }
  let res;
  try {
    res = await fetch(`${apiBase()}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch (err) {
    setApiOnline(false);
    runtime.pollBackoffMs = Math.min(runtime.pollBackoffMs * 2, 60_000);
    setThrottle(`Network error — backing off polls (${Math.round(runtime.pollBackoffMs / 1000)}s)`, Date.now() + runtime.pollBackoffMs);
    schedulePolling();
    throw new Error(err.message || "Network error — API unreachable");
  }
  if (path === "/health" || res.ok || res.status !== 0) {
    if (res.status !== 0) setApiOnline(true);
  }
  const text = await res.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (res.status === 429) {
    const retryAfter = Number(res.headers.get("Retry-After") || 5);
    const waitMs = (Number.isFinite(retryAfter) ? retryAfter : 5) * 1000;
    runtime.throttledUntil = Date.now() + waitMs;
    runtime.pollBackoffMs = Math.max(runtime.pollBackoffMs, waitMs);
    setThrottle(`HTTP 429 — UI throttling polls for ~${Math.round(waitMs / 1000)}s (security policy unchanged)`, runtime.throttledUntil);
    schedulePolling();
    const msg = data && data.error ? data.error : "429 Too Many Requests";
    throw new Error(msg);
  }
  if (!res.ok) {
    const msg = data && data.error ? data.error : `${res.status} ${res.statusText}`;
    throw new Error(msg);
  }
  if (runtime.pollBackoffMs > BASE_POLL_MS && path === "/health") {
    runtime.pollBackoffMs = BASE_POLL_MS;
    clearThrottleIfIdle();
    schedulePolling();
  }
  return data;
}

function showOut(el, data) {
  el.hidden = false;
  el.textContent = typeof data === "string" ? data : JSON.stringify(data, null, 2);
}

function loadHistory() {
  try {
    return JSON.parse(localStorage.getItem(HISTORY_KEY) || "[]");
  } catch {
    return [];
  }
}

function saveHistory(ids) {
  const uniq = [...new Set(ids)].slice(0, 100);
  localStorage.setItem(HISTORY_KEY, JSON.stringify(uniq));
}

function rememberJob(id) {
  if (!id) return;
  const hist = loadHistory();
  hist.unshift(String(id));
  saveHistory(hist);
}

function themeById(id) {
  return THEMES.find((t) => t.id === id) || THEMES[0];
}

function osPrefersDark() {
  return window.matchMedia && window.matchMedia("(prefers-color-scheme: dark)").matches;
}

function isOsSyncEnabled() {
  return localStorage.getItem(THEME_SYNC_KEY) === "1";
}

function applyTheme(id, { persist = true } = {}) {
  const theme = themeById(id);
  const link = $("#theme-sheet");
  link.href = `/ui/css/themes/${theme.id}.css`;
  document.documentElement.dataset.theme = theme.id;
  document.documentElement.dataset.scheme = theme.scheme;
  if (persist) {
    localStorage.setItem(THEME_KEY, theme.id);
    if (theme.scheme === "dark") localStorage.setItem(THEME_DARK_KEY, theme.id);
    else localStorage.setItem(THEME_LIGHT_KEY, theme.id);
  }
  const select = $("#theme-select");
  if (select) select.value = theme.id;
}

function resolveThemeId() {
  if (isOsSyncEnabled()) {
    const dark = osPrefersDark();
    if (dark) return localStorage.getItem(THEME_DARK_KEY) || "night-slate";
    return localStorage.getItem(THEME_LIGHT_KEY) || localStorage.getItem(THEME_KEY) || "slate-teal";
  }
  return localStorage.getItem(THEME_KEY) || "slate-teal";
}

function initThemes() {
  const select = $("#theme-select");
  select.innerHTML = THEMES.map((t) => `<option value="${t.id}">${t.label}</option>`).join("");
  const sync = $("#theme-os-sync");
  if (sync) sync.checked = isOsSyncEnabled();
  applyTheme(resolveThemeId());
  select.addEventListener("change", () => {
    applyTheme(select.value);
  });
  sync?.addEventListener("change", () => {
    localStorage.setItem(THEME_SYNC_KEY, sync.checked ? "1" : "0");
    applyTheme(resolveThemeId());
  });
  if (window.matchMedia) {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => {
      if (isOsSyncEnabled()) applyTheme(resolveThemeId());
    };
    if (mq.addEventListener) mq.addEventListener("change", onChange);
    else if (mq.addListener) mq.addListener(onChange);
  }
}

function switchPanel(name) {
  $$(".nav-btn").forEach((btn) => {
    btn.classList.toggle("is-active", btn.dataset.panel === name);
  });
  $$(".panel").forEach((panel) => {
    const active = panel.id === `panel-${name}`;
    panel.classList.toggle("is-active", active);
    panel.hidden = !active;
  });
  if (name === "monitor") refreshMonitor();
  if (name === "nodes") {
    refreshMesh();
    refreshMeshHealth();
  }
  if (name === "jobs") {
    refreshJobsTable();
    refreshTemplates();
  }
  if (name === "host") {
    refreshHost();
    refreshDrain();
  }
}

function loadTemplates() {
  try {
    return JSON.parse(localStorage.getItem(TEMPLATES_KEY) || "[]");
  } catch {
    return [];
  }
}

function saveTemplates(list) {
  localStorage.setItem(TEMPLATES_KEY, JSON.stringify(list.slice(0, 40)));
}

function refreshTemplates() {
  const select = $("#template-select");
  if (!select) return;
  const list = loadTemplates();
  select.innerHTML = list.length
    ? list.map((t, i) => `<option value="${i}">${t.name}</option>`).join("")
    : `<option value="">No presets yet</option>`;
}

function readJobForm() {
  const vram = $("#job-vram").value.trim();
  const timeout = $("#job-timeout").value.trim();
  return {
    kind: $("#job-kind").value,
    routing: $("#job-routing").value,
    model_id: $("#job-model").value.trim(),
    revision_hash: $("#job-rev").value.trim(),
    tokenizer_revision: $("#job-tok").value.trim() || null,
    precision: $("#job-precision").value.trim() || "fp16",
    payload: $("#job-payload").value,
    requested_vram_mb: vram ? Number(vram) : null,
    timeout_secs: timeout ? Number(timeout) : null,
  };
}

function applyJobForm(t) {
  if (!t) return;
  if (t.kind) $("#job-kind").value = t.kind;
  if (t.routing) $("#job-routing").value = t.routing;
  if (t.model_id) $("#job-model").value = t.model_id;
  if (t.revision_hash) $("#job-rev").value = t.revision_hash;
  $("#job-tok").value = t.tokenizer_revision || "";
  $("#job-precision").value = t.precision || "fp16";
  if (t.payload != null) $("#job-payload").value = t.payload;
  $("#job-vram").value = t.requested_vram_mb ?? "";
  $("#job-timeout").value = t.timeout_secs ?? "";
}

function parseHordeJoinUri(raw) {
  const text = (raw || "").trim();
  if (!text) return null;
  let url;
  try {
    if (text.startsWith("horde://")) {
      url = new URL(text.replace(/^horde:\/\//, "https://horde.local/"));
    } else if (text.includes("addr=") && text.includes("token=")) {
      url = new URL(text.includes("://") ? text : `https://horde.local/?${text.replace(/^\?/, "")}`);
    } else {
      return null;
    }
  } catch {
    return null;
  }
  const addr = url.searchParams.get("addr");
  const token = url.searchParams.get("token");
  if (!addr || !token) return null;
  return { addr, token, expires: url.searchParams.get("expires") };
}

function formatRemaining(ms) {
  if (ms <= 0) return "expired";
  const s = Math.floor(ms / 1000);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  if (h > 0) return `${h}h ${m}m ${sec}s`;
  if (m > 0) return `${m}m ${sec}s`;
  return `${sec}s`;
}

function updateInviteCountdown() {
  const el = $("#invite-ttl");
  if (!el) return;
  const inv = runtime.lastInvite;
  if (!inv?.expires_at) {
    el.textContent = "No active invite TTL in this session.";
    return;
  }
  const ends = Date.parse(inv.expires_at);
  if (Number.isNaN(ends)) {
    el.textContent = `Expires at ${inv.expires_at}`;
    return;
  }
  const left = ends - Date.now();
  el.textContent =
    left <= 0
      ? `Invite expired at ${inv.expires_at} — renew to mint a fresh token.`
      : `Expires in ${formatRemaining(left)} (${inv.expires_at})`;
}

function startInviteCountdown() {
  if (runtime.inviteTimer) clearInterval(runtime.inviteTimer);
  updateInviteCountdown();
  runtime.inviteTimer = setInterval(updateInviteCountdown, 1000);
}

function rememberInvite(out) {
  if (!out || !out.expires_at) return;
  runtime.lastInvite = {
    expires_at: out.expires_at,
    purpose: out.purpose || $("#mint-purpose")?.value,
    ttl_secs: Number($("#mint-ttl")?.value) || 3600,
    note: $("#mint-note")?.value?.trim() || "",
    token_id: out.token_id,
  };
  startInviteCountdown();
}

function renderInviteQr(uri) {
  const box = $("#invite-qr");
  const share = $("#invite-share");
  const input = $("#invite-join-uri");
  if (!uri) {
    share.hidden = true;
    return;
  }
  share.hidden = false;
  input.value = uri;
  box.innerHTML = "";
  if (typeof QRCode !== "undefined") {
    // eslint-disable-next-line no-new
    new QRCode(box, { text: uri, width: 164, height: 164, correctLevel: QRCode.CorrectLevel.M });
  } else {
    box.textContent = "QR library unavailable — copy the link instead.";
  }
}

function extractPreviewText(result) {
  if (result == null) return null;
  if (typeof result === "string") return result;
  if (typeof result.output === "string") return result.output;
  if (result.output != null) {
    try {
      return JSON.stringify(result.output, null, 2);
    } catch {
      return String(result.output);
    }
  }
  try {
    return JSON.stringify(result, null, 2);
  } catch {
    return String(result);
  }
}

function showResultPreview(result) {
  const pane = $("#result-preview");
  const body = $("#preview-body");
  const meta = $("#preview-meta");
  if (!pane || !body) return;
  const text = extractPreviewText(result);
  if (text == null) {
    pane.hidden = true;
    return;
  }
  const bytes = new TextEncoder().encode(text).length;
  const looksJson =
    (text.trim().startsWith("{") && text.trim().endsWith("}")) ||
    (text.trim().startsWith("[") && text.trim().endsWith("]"));
  if (bytes > PREVIEW_MAX_CHARS) {
    pane.hidden = false;
    meta.textContent = `Output is ${bytes} bytes — too large for inline preview. Use Download.`;
    body.textContent = text.slice(0, 2000) + "\n… [truncated]";
    runtime.previewBlob = text;
    return;
  }
  pane.hidden = false;
  meta.textContent = `${looksJson ? "JSON" : "text"} · ${bytes} bytes`;
  body.textContent = text;
  runtime.previewBlob = text;
  showOut($("#job-track-out"), result);
}

function downloadBlob(filename, text, mime) {
  const blob = new Blob([text], { type: mime });
  const a = document.createElement("a");
  a.href = URL.createObjectURL(blob);
  a.download = filename;
  a.click();
  URL.revokeObjectURL(a.href);
}

function jobsAndPeersToCsv(snap) {
  const lines = ["section,key,value"];
  const push = (section, key, value) => {
    const v = String(value ?? "").replace(/"/g, '""');
    lines.push(`${section},${key},"${v}"`);
  };
  push("meta", "exported_at", snap.exported_at);
  push("meta", "node_id", snap.node_id);
  for (const j of snap.jobs || []) {
    push("job", j.id, `${j.status}|${j.kind}|${j.updated_at || ""}|peer=${j.assigned_peer_id || ""}|server=${j.assigned_server_id || ""}`);
  }
  for (const p of snap.peer_stats || []) {
    push(
      "peer_stats",
      p.node_id,
      `accept=${p.accepted_results}|reject=${p.rejected_results}|weight=${p.fair_share_weight}`
    );
  }
  for (const p of snap.mesh?.peers || []) {
    push("mesh_peer", p.node_id, `${p.addr}|rtt=${p.rtt_ms ?? ""}|last_seen=${p.last_seen || ""}`);
  }
  return lines.join("\n");
}

async function doExport(format) {
  try {
    const snap = await api("/v1/admin/export");
    const stamp = new Date().toISOString().replace(/[:.]/g, "-");
    if (format === "json") {
      downloadBlob(`horde-export-${stamp}.json`, JSON.stringify(snap, null, 2), "application/json");
    } else {
      downloadBlob(`horde-export-${stamp}.csv`, jobsAndPeersToCsv(snap), "text/csv");
    }
    showOut($("#export-out"), { ok: true, format, jobs: (snap.jobs || []).length });
  } catch (err) {
    showOut($("#export-out"), String(err.message || err));
  }
}

async function refreshDrain() {
  if (!getKey()) return;
  try {
    const d = await api("/v1/admin/drain");
    $("#drain-state").textContent = d.draining ? (d.ready_to_exit ? "ready to exit" : "draining") : "active";
    $("#drain-inflight").textContent = String(d.in_flight ?? "—");
    $("#drain-note").textContent = d.note || "—";
  } catch {
    /* ignore */
  }
}

async function refreshMeshHealth() {
  const el = $("#mesh-health-strip");
  if (!el) return;
  if (!getKey()) {
    el.textContent = "Save an API key to load mesh health.";
    return;
  }
  try {
    const health = await api("/v1/admin/mesh/health");
    const peers = health.peers || [];
    if (!peers.length) {
      el.textContent = "No connected peers.";
      return;
    }
    runtime.slowPeers = peers
      .filter((p) => p.slow || (p.rtt_ms != null && p.rtt_ms >= SLOW_PEER_RTT_MS))
      .map((p) => p.node_id);
    clearThrottleIfIdle();
    if (runtime.slowPeers.length) setThrottle(null);
    el.innerHTML = peers
      .map((p) => {
        const acceptPct =
          p.accept_rate == null ? "—" : `${Math.round(p.accept_rate * 100)}%`;
        const rejectPct =
          p.reject_rate == null ? "—" : `${Math.round(p.reject_rate * 100)}%`;
        const slow = p.slow || (p.rtt_ms != null && p.rtt_ms >= SLOW_PEER_RTT_MS);
        const muted = !!p.muted;
        return `<div class="health-peer" data-slow="${slow}" data-muted="${muted}">
          <div class="peer-id">${p.node_id}${muted ? " (muted)" : ""}</div>
          <div class="peer-metrics">
            <span>${p.addr || ""}</span>
            <span>last-seen ${p.last_seen || "—"}</span>
            <span data-rtt>RTT ${p.rtt_ms != null ? p.rtt_ms + " ms" : "—"}${slow ? " · slow" : ""}</span>
            <span>accept ${acceptPct}</span>
            <span>reject ${rejectPct}</span>
            ${muted ? "<span>pause accept</span>" : ""}
          </div>
        </div>`;
      })
      .join("");
  } catch (err) {
    el.textContent = String(err.message || err);
  }
}

function renderTimeline(events) {
  const ol = $("#job-timeline-view");
  if (!events || !events.length) {
    ol.hidden = true;
    ol.innerHTML = "";
    return;
  }
  ol.hidden = false;
  ol.innerHTML = events
    .map((e) => {
      const who = e.peer_id
        ? `peer ${e.peer_id}`
        : e.server_id
          ? `server ${e.server_id}`
          : "—";
      const note = e.note ? ` · ${e.note}` : "";
      return `<li><strong>${e.status}</strong> @ ${e.at || "—"} · ${who}${note}</li>`;
    })
    .join("");
}

/* ---- Host ---- */

async function refreshHost() {
  try {
    const health = await api("/health", { auth: false });
    $("#host-health").textContent = health.status || "ok";
    setApiOnline(true);
  } catch (err) {
    $("#host-health").textContent = "unreachable";
    setApiOnline(false);
    return;
  }

  try {
    const meta = await api("/v1/meta", { auth: false });
    $("#host-mode").textContent = meta.mode || "—";
    $("#host-node-id").textContent = meta.node_id || "—";
  } catch {
    /* meta optional if API up */
  }

  if (!getKey()) return;

  try {
    const [status, config, servers] = await Promise.all([
      api("/v1/admin/status"),
      api("/v1/admin/config"),
      api("/v1/admin/servers"),
    ]);
    $("#host-members").textContent = String(status.member_count ?? "—");
    $("#host-jobs").textContent = String(status.job_count ?? "—");
    $("#host-mode").textContent = status.mode || $("#host-mode").textContent;
    $("#host-node-id").textContent = status.node_id || $("#host-node-id").textContent;
    $("#host-bind").textContent = config.bind || "—";
    $("#host-data-dir").textContent = config.data_dir || "—";
    $("#host-mesh").textContent = config.mesh_enabled
      ? `enabled (${config.mesh_bind || "?"})`
      : "disabled (set bind below or HORDE_MESH_BIND at start)";
    $("#host-ui-url").textContent = `http://${config.bind}/ui/`;
    if (config.mesh_bind && !$("#mesh-bind-input").value) {
      $("#mesh-bind-input").value = config.mesh_bind;
    }
    if (config.mesh_bind && !$("#nodes-mesh-bind-input").value) {
      $("#nodes-mesh-bind-input").value = config.mesh_bind;
    }

    const select = $("#limits-server");
    const current = select.value;
    select.innerHTML = servers
      .map((s) => `<option value="${s.server_id}">${s.server_id} (${s.hostname})</option>`)
      .join("");
    if (current && [...select.options].some((o) => o.value === current)) {
      select.value = current;
    }
    fillLimitsForm(servers.find((s) => s.server_id === select.value) || servers[0]);
  } catch (err) {
    setConn("bad", "auth?");
    console.warn(err);
  }
}

function fillLimitsForm(server) {
  if (!server) return;
  const lim = server.limits || {};
  $("#limits-gpu").value = lim.gpu_percent ?? 100;
  $("#limits-start").value = lim.time_window?.start_hour ?? 0;
  $("#limits-end").value = lim.time_window?.end_hour ?? 24;
  $("#limits-disk").value = lim.disk_budget_mb ?? 50000;
  $("#limits-network").checked = !!lim.accept_network_jobs;
  $("#limits-enabled").checked = lim.enabled !== false;
}

/* ---- Nodes ---- */

async function refreshMesh() {
  const out = $("#mesh-status");
  if (!getKey()) {
    out.textContent = "Save an API key to load mesh status.";
    return;
  }
  try {
    const status = await api("/v1/admin/mesh/status");
    showOut(out, status);
    if (status.bind) {
      $("#nodes-mesh-bind-input").value = status.bind;
      if (!$("#mesh-bind-input").value) {
        $("#mesh-bind-input").value = status.bind;
      }
    }
    if (status.public_dht) {
      $("#dht-enabled").checked = !!status.public_dht.enabled;
    }
  } catch (err) {
    out.hidden = false;
    out.textContent = String(err.message || err);
  }
  try {
    const bl = await api("/v1/admin/mesh/blocklist");
    showOut($("#block-out"), bl);
  } catch {
    /* mesh may be off */
  }
  try {
    const muted = await api("/v1/admin/mesh/mute");
    showOut($("#mute-out"), muted);
  } catch {
    /* mesh may be off */
  }
  refreshMeshHealth();
}

/* ---- Jobs ---- */

async function refreshJobsTable() {
  const tbody = $("#jobs-table tbody");
  tbody.innerHTML = "";
  const localIds = loadHistory();
  let serverJobs = [];
  if (getKey()) {
    try {
      serverJobs = await api("/v1/jobs?limit=50");
    } catch (err) {
      console.warn(err);
    }
  }
  const byId = new Map(serverJobs.map((j) => [String(j.id), j]));
  for (const id of localIds) {
    if (!byId.has(id)) byId.set(id, { id, status: "local-only", kind: "—", updated_at: "" });
  }
  const rows = [...byId.values()];
  if (!rows.length) {
    tbody.innerHTML = `<tr><td colspan="5">No jobs yet.</td></tr>`;
    return;
  }
  tbody.innerHTML = rows
    .map((j) => {
      const id = typeof j.id === "object" ? j.id : j.id;
      return `<tr>
        <td class="mono">${id}</td>
        <td>${j.kind || "—"}</td>
        <td>${j.status || "—"}</td>
        <td>${j.updated_at || "—"}</td>
        <td><button type="button" class="linkish" data-track="${id}">Open</button></td>
      </tr>`;
    })
    .join("");
  tbody.querySelectorAll("[data-track]").forEach((btn) => {
    btn.addEventListener("click", () => {
      $("#track-id").value = btn.dataset.track;
      pollJob();
    });
  });
}

async function pollJob() {
  const id = $("#track-id").value.trim();
  if (!id) return;
  try {
    const job = await api(`/v1/jobs/${encodeURIComponent(id)}`);
    rememberJob(id);
    showOut($("#job-track-out"), job);
    renderTimeline(job.timeline || []);
    refreshJobsTable();
  } catch (err) {
    showOut($("#job-track-out"), String(err.message || err));
  }
}

async function loadTimeline() {
  const id = $("#track-id").value.trim();
  if (!id) return;
  try {
    const data = await api(`/v1/jobs/${encodeURIComponent(id)}/timeline`);
    renderTimeline(data.timeline || []);
    showOut($("#job-track-out"), data);
  } catch (err) {
    showOut($("#job-track-out"), String(err.message || err));
  }
}

/* ---- Monitor ---- */

async function refreshMonitor() {
  if (!getKey()) return;
  try {
    const [status, reputation, jobs] = await Promise.all([
      api("/v1/admin/status"),
      api("/v1/admin/reputation"),
      api("/v1/jobs?limit=30"),
    ]);
    $("#mon-members").textContent = String(status.member_count ?? "—");
    $("#mon-jobs").textContent = String(status.job_count ?? "—");
    $("#mon-mode").textContent = status.mode || "—";
    const peers = status.mesh?.peer_count ?? status.mesh?.peers?.length ?? "—";
    $("#mon-peers").textContent = String(peers);

    const mt = $("#mon-members-table tbody");
    mt.innerHTML = (status.members || [])
      .map(
        (m) => `<tr>
        <td class="mono">${m.server_id}</td>
        <td>${m.hostname || ""}</td>
        <td>${m.limits?.gpu_percent ?? ""}</td>
        <td>${m.limits?.enabled ? "yes" : "no"}</td>
        <td>${m.limits?.accept_network_jobs ? "yes" : "no"}</td>
      </tr>`
      )
      .join("") || `<tr><td colspan="5">No members</td></tr>`;

    showOut($("#mon-reputation"), reputation);

    const jt = $("#mon-jobs-table tbody");
    jt.innerHTML = (jobs || [])
      .map((j) => {
        const id = j.id;
        return `<tr>
          <td class="mono">${id}</td>
          <td>${j.kind}</td>
          <td>${j.status}</td>
          <td>${j.routing || ""}</td>
          <td>${j.updated_at || ""}</td>
        </tr>`;
      })
      .join("") || `<tr><td colspan="5">No recent jobs</td></tr>`;
  } catch (err) {
    console.warn(err);
  }
}

async function loadSupportHint() {
  if (!getKey()) return;
  try {
    const matrix = await api("/v1/admin/support-matrix");
    const kinds = (matrix.workloads || []).map((w) => w.kind || w.name || JSON.stringify(w));
    $("#support-hint").textContent = kinds.length
      ? `Support matrix kinds: ${kinds.join(", ")}`
      : "Support matrix loaded.";
  } catch {
    $("#support-hint").textContent = "Could not load support matrix.";
  }
}

function wireForms() {
  $("#save-key").addEventListener("click", () => {
    const key = $("#api-key").value.trim();
    if (key) localStorage.setItem(STORAGE_KEY, key);
    else localStorage.removeItem(STORAGE_KEY);
    if (key) markChecklist("key", true);
    refreshHost();
    loadSupportHint();
    refreshChecklistFromDiagnostics();
  });

  $("#offline-retry")?.addEventListener("click", async () => {
    try {
      await api("/health", { auth: false });
      setApiOnline(true);
      runtime.pollBackoffMs = BASE_POLL_MS;
      clearThrottleIfIdle();
      schedulePolling();
      refreshHost();
    } catch {
      setApiOnline(false);
    }
  });

  $("#checklist-dismiss")?.addEventListener("click", () => {
    localStorage.setItem(CHECKLIST_DISMISS_KEY, "1");
    const banner = $("#checklist-banner");
    if (banner) banner.hidden = true;
  });
  $("#checklist-refresh")?.addEventListener("click", () => refreshChecklistFromDiagnostics());

  const saved = localStorage.getItem(STORAGE_KEY);
  if (saved) $("#api-key").value = saved;

  $$(".nav-btn").forEach((btn) => {
    btn.addEventListener("click", () => switchPanel(btn.dataset.panel));
  });

  $("#host-refresh").addEventListener("click", refreshHost);

  async function applyMeshBind(inputEl, outEl) {
    const bind = inputEl.value.trim();
    if (!bind) return;
    try {
      const out = await api("/v1/admin/mesh/bind", {
        method: "POST",
        body: { bind },
      });
      showOut(outEl, out);
      if (out.mesh_bind) {
        $("#mesh-bind-input").value = out.mesh_bind;
        $("#nodes-mesh-bind-input").value = out.mesh_bind;
      }
      markChecklist("bind", true);
      refreshHost();
      refreshMesh();
      refreshChecklistFromDiagnostics();
    } catch (err) {
      showOut(outEl, String(err.message || err));
    }
  }

  $("#mesh-bind-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    await applyMeshBind($("#mesh-bind-input"), $("#mesh-bind-out"));
  });

  $("#nodes-mesh-bind-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    await applyMeshBind($("#nodes-mesh-bind-input"), $("#nodes-mesh-bind-out"));
  });

  $("#limits-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const serverId = $("#limits-server").value;
    const body = {
      gpu_percent: Number($("#limits-gpu").value),
      time_window: {
        start_hour: Number($("#limits-start").value),
        end_hour: Number($("#limits-end").value),
      },
      disk_budget_mb: Number($("#limits-disk").value),
      accept_network_jobs: $("#limits-network").checked,
      enabled: $("#limits-enabled").checked,
    };
    try {
      const snap = await api(`/v1/admin/servers/${encodeURIComponent(serverId)}/limits`, {
        method: "POST",
        body,
      });
      showOut($("#limits-out"), snap);
      refreshHost();
    } catch (err) {
      showOut($("#limits-out"), String(err.message || err));
    }
  });

  $("#limits-server").addEventListener("change", async () => {
    try {
      const servers = await api("/v1/admin/servers");
      fillLimitsForm(servers.find((s) => s.server_id === $("#limits-server").value));
    } catch {
      /* ignore */
    }
  });

  $("#api-key-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    try {
      const out = await api("/v1/admin/api-keys", {
        method: "POST",
        body: { label: $("#api-key-label").value.trim() || "operator" },
      });
      showOut($("#api-key-out"), out);
    } catch (err) {
      showOut($("#api-key-out"), String(err.message || err));
    }
  });

  $("#mesh-refresh").addEventListener("click", refreshMesh);

  async function mintInvite({ renew = false } = {}) {
    const body = {
      purpose: $("#mint-purpose").value,
      ttl_secs: Number($("#mint-ttl").value) || 3600,
    };
    if (renew && runtime.lastInvite) {
      body.purpose = runtime.lastInvite.purpose || body.purpose;
      body.ttl_secs = runtime.lastInvite.ttl_secs || body.ttl_secs;
    }
    const note = $("#mint-note").value.trim();
    if (note) body.note = note;
    else if (renew && runtime.lastInvite?.note) body.note = runtime.lastInvite.note;
    const out = await api("/v1/admin/invites", { method: "POST", body });
    showOut($("#mint-out"), out);
    rememberInvite(out);
    markChecklist("mint", true);
    let uri = out.join_uri;
    if (!uri && out.token) {
      const addr =
        out.mesh_addr ||
        $("#nodes-mesh-bind-input").value.trim() ||
        $("#mesh-bind-input").value.trim();
      if (addr) {
        const exp = encodeURIComponent(out.expires_at || "");
        uri = `horde://join?addr=${encodeURIComponent(addr)}&token=${encodeURIComponent(out.token)}&expires=${exp}`;
      }
    }
    renderInviteQr(uri || null);
    return out;
  }

  $("#mint-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    try {
      await mintInvite();
    } catch (err) {
      showOut($("#mint-out"), String(err.message || err));
      renderInviteQr(null);
    }
  });

  $("#invite-copy")?.addEventListener("click", async () => {
    await copyInviteLink();
  });

  $("#invite-renew")?.addEventListener("click", async () => {
    try {
      await mintInvite({ renew: true });
    } catch (err) {
      showOut($("#mint-out"), String(err.message || err));
    }
  });

  async function copyInviteLink() {
    const uri = $("#invite-join-uri")?.value;
    if (!uri) {
      showOut($("#mint-out"), "No invite link to copy — mint an invite first.");
      return false;
    }
    try {
      await navigator.clipboard.writeText(uri);
      showOut($("#mint-out"), { copied: uri });
      return true;
    } catch {
      $("#invite-join-uri").select();
      return false;
    }
  }

  function applyJoinPaste() {
    const parsed = parseHordeJoinUri($("#join-paste").value);
    if (!parsed) return false;
    $("#join-addr").value = parsed.addr;
    $("#join-token").value = parsed.token;
    return true;
  }

  $("#join-parse")?.addEventListener("click", () => {
    if (!applyJoinPaste()) {
      showOut($("#join-out"), "Could not parse horde:// join link");
    } else {
      showOut($("#join-out"), { parsed: true, addr: $("#join-addr").value });
    }
  });

  $("#join-paste")?.addEventListener("change", applyJoinPaste);

  $("#join-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    applyJoinPaste();
    try {
      const out = await api("/v1/admin/mesh/join", {
        method: "POST",
        body: {
          peer_addr: $("#join-addr").value.trim(),
          token: $("#join-token").value.trim(),
        },
      });
      showOut($("#join-out"), out);
      markChecklist("join", true);
      refreshMesh();
      refreshChecklistFromDiagnostics();
    } catch (err) {
      showOut($("#join-out"), String(err.message || err));
    }
  });

  $("#bootstrap-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const peers = $("#bootstrap-peers")
      .value.split(",")
      .map((s) => s.trim())
      .filter(Boolean)
      .map((addr) => ({ addr, note: "ui bootstrap" }));
    try {
      const out = await api("/v1/admin/mesh/bootstrap", { method: "POST", body: { peers } });
      showOut($("#bootstrap-out"), out);
      refreshMesh();
    } catch (err) {
      showOut($("#bootstrap-out"), String(err.message || err));
    }
  });

  $("#block-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    try {
      const out = await api("/v1/admin/mesh/blocklist", {
        method: "POST",
        body: { node_id: $("#block-id").value.trim() },
      });
      showOut($("#block-out"), out);
      refreshMesh();
    } catch (err) {
      showOut($("#block-out"), String(err.message || err));
    }
  });

  $("#unblock-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const id = $("#unblock-id").value.trim();
    try {
      const out = await api(`/v1/admin/mesh/blocklist/${encodeURIComponent(id)}`, {
        method: "DELETE",
      });
      showOut($("#block-out"), out);
      refreshMesh();
    } catch (err) {
      showOut($("#block-out"), String(err.message || err));
    }
  });

  $("#mute-form")?.addEventListener("submit", async (e) => {
    e.preventDefault();
    try {
      const out = await api("/v1/admin/mesh/mute", {
        method: "POST",
        body: { node_id: $("#mute-id").value.trim() },
      });
      showOut($("#mute-out"), out);
      refreshMesh();
    } catch (err) {
      showOut($("#mute-out"), String(err.message || err));
    }
  });

  $("#unmute-form")?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const id = $("#unmute-id").value.trim();
    try {
      const out = await api(`/v1/admin/mesh/mute/${encodeURIComponent(id)}`, {
        method: "DELETE",
      });
      showOut($("#mute-out"), out);
      refreshMesh();
    } catch (err) {
      showOut($("#mute-out"), String(err.message || err));
    }
  });

  $("#dht-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    try {
      const out = await api("/v1/admin/mesh/dht", {
        method: "POST",
        body: { enabled: $("#dht-enabled").checked },
      });
      showOut($("#dht-out"), out);
    } catch (err) {
      showOut($("#dht-out"), String(err.message || err));
    }
  });

  $("#template-form")?.addEventListener("submit", (e) => {
    e.preventDefault();
    const name = $("#template-name").value.trim();
    if (!name) return;
    const list = loadTemplates();
    list.unshift({ name, ...readJobForm() });
    saveTemplates(list);
    $("#template-name").value = "";
    refreshTemplates();
  });
  $("#template-apply")?.addEventListener("click", () => {
    const list = loadTemplates();
    const i = Number($("#template-select").value);
    applyJobForm(list[i]);
  });
  $("#template-delete")?.addEventListener("click", () => {
    const list = loadTemplates();
    const i = Number($("#template-select").value);
    if (Number.isNaN(i) || i < 0) return;
    list.splice(i, 1);
    saveTemplates(list);
    refreshTemplates();
  });

  $("#job-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const form = readJobForm();
    const body = {
      fingerprint: {
        model_id: form.model_id,
        revision_hash: form.revision_hash,
        precision: form.precision,
      },
      kind: form.kind,
      payload: form.payload,
      routing: form.routing,
    };
    if (form.tokenizer_revision) body.fingerprint.tokenizer_revision = form.tokenizer_revision;
    if (form.requested_vram_mb) body.requested_vram_mb = form.requested_vram_mb;
    try {
      const out = await api("/v1/jobs", { method: "POST", body });
      showOut($("#job-create-out"), out);
      if (out.id) {
        $("#track-id").value = out.id;
        rememberJob(out.id);
        renderTimeline(out.timeline || []);
        refreshJobsTable();
        markChecklist("submit", true);
        refreshChecklistFromDiagnostics();
      }
    } catch (err) {
      showOut($("#job-create-out"), String(err.message || err));
    }
  });

  $("#job-track-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    await pollJob();
  });

  $("#job-cancel").addEventListener("click", async () => {
    const id = $("#track-id").value.trim();
    try {
      showOut($("#job-track-out"), await api(`/v1/jobs/${encodeURIComponent(id)}/cancel`, { method: "POST" }));
      refreshJobsTable();
    } catch (err) {
      showOut($("#job-track-out"), String(err.message || err));
    }
  });
  $("#job-accept").addEventListener("click", async () => {
    const id = $("#track-id").value.trim();
    try {
      showOut($("#job-track-out"), await api(`/v1/jobs/${encodeURIComponent(id)}/accept`, { method: "POST" }));
    } catch (err) {
      showOut($("#job-track-out"), String(err.message || err));
    }
  });
  $("#job-reject").addEventListener("click", async () => {
    const id = $("#track-id").value.trim();
    try {
      showOut($("#job-track-out"), await api(`/v1/jobs/${encodeURIComponent(id)}/reject`, { method: "POST" }));
    } catch (err) {
      showOut($("#job-track-out"), String(err.message || err));
    }
  });
  $("#job-result").addEventListener("click", async () => {
    const id = $("#track-id").value.trim();
    try {
      const result = await api(`/v1/jobs/${encodeURIComponent(id)}/result`);
      showResultPreview(result);
    } catch (err) {
      showOut($("#job-track-out"), String(err.message || err));
    }
  });
  $("#preview-download")?.addEventListener("click", () => {
    if (!runtime.previewBlob) return;
    const id = $("#track-id").value.trim() || "result";
    downloadBlob(`horde-result-${id}.txt`, runtime.previewBlob, "text/plain;charset=utf-8");
  });
  $("#preview-hide")?.addEventListener("click", () => {
    const pane = $("#result-preview");
    if (pane) pane.hidden = true;
  });
  $("#job-timeline")?.addEventListener("click", loadTimeline);

  $("#jobs-refresh").addEventListener("click", refreshJobsTable);
  $("#jobs-clear-local").addEventListener("click", () => {
    localStorage.removeItem(HISTORY_KEY);
    refreshJobsTable();
  });

  $("#mon-refresh").addEventListener("click", refreshMonitor);
  $("#mon-export-json")?.addEventListener("click", () => doExport("json"));
  $("#mon-export-csv")?.addEventListener("click", () => doExport("csv"));
  $("#export-json")?.addEventListener("click", () => doExport("json"));
  $("#export-csv")?.addEventListener("click", () => doExport("csv"));

  $("#drain-start")?.addEventListener("click", async () => {
    if (!confirm("Begin graceful drain? New jobs will be rejected and the process will exit when in-flight work finishes.")) {
      return;
    }
    try {
      const out = await api("/v1/admin/drain", { method: "POST" });
      showOut($("#drain-out"), out);
      refreshDrain();
    } catch (err) {
      showOut($("#drain-out"), String(err.message || err));
    }
  });

  $("#diag-form")?.addEventListener("submit", async (e) => {
    e.preventDefault();
    const keyPresent = !!getKey();
    const list = $("#diag-checks");
    if (!keyPresent) {
      list.hidden = false;
      list.innerHTML = `<li data-ok="false"><strong>api_key</strong> — paste the key from the startup log first</li>`;
      showOut($("#diag-out"), { ok: false, reason: "api key missing in UI" });
      return;
    }
    try {
      const out = await api("/v1/admin/diagnostics/join", {
        method: "POST",
        body: {
          peer_addr: $("#diag-addr").value.trim() || null,
          token: $("#diag-token").value.trim() || null,
          client_time: new Date().toISOString(),
        },
      });
      list.hidden = false;
      list.innerHTML = (out.checks || [])
        .map(
          (c) =>
            `<li data-ok="${c.ok}"><strong>${c.id}</strong> — ${c.detail}</li>`
        )
        .join("");
      showOut($("#diag-out"), out);
    } catch (err) {
      showOut($("#diag-out"), String(err.message || err));
    }
  });

  // Auto-fill tokenize defaults when kind changes
  $("#job-kind").addEventListener("change", () => {
    const kind = $("#job-kind").value;
    if (kind === "tokenize") {
      $("#job-model").value = "stub/tokenizer";
      $("#job-tok").value = $("#job-tok").value || "tok-rev-1";
      $("#job-payload").value =
        '{\n  "text": "hello horde tokenize",\n  "seed": 1,\n  "chunk_tokens": 8\n}';
    } else if (kind === "stub_echo") {
      $("#job-model").value = "stub/echo";
      $("#job-payload").value = "hello horde";
    }
  });
}

function loadChecklistState() {
  try {
    return JSON.parse(localStorage.getItem(CHECKLIST_STATE_KEY) || "{}");
  } catch {
    return {};
  }
}

function saveChecklistState(state) {
  localStorage.setItem(CHECKLIST_STATE_KEY, JSON.stringify(state));
}

function markChecklist(step, done) {
  const state = loadChecklistState();
  state[step] = !!done;
  saveChecklistState(state);
  renderChecklist();
}

function renderChecklist() {
  const banner = $("#checklist-banner");
  if (!banner) return;
  if (localStorage.getItem(CHECKLIST_DISMISS_KEY) === "1") {
    banner.hidden = true;
    return;
  }
  const state = loadChecklistState();
  const steps = ["key", "bind", "mint", "join", "submit"];
  let allDone = true;
  for (const step of steps) {
    const li = $(`#setup-checklist li[data-step="${step}"]`);
    const done = !!state[step];
    if (!done) allDone = false;
    if (li) {
      li.dataset.done = done ? "true" : "false";
      const mark = li.querySelector("span");
      if (mark) mark.textContent = done ? "●" : "○";
    }
  }
  banner.hidden = allDone;
}

async function refreshChecklistFromDiagnostics() {
  if (localStorage.getItem(CHECKLIST_DISMISS_KEY) === "1") {
    renderChecklist();
    return;
  }
  if (getKey()) markChecklist("key", true);
  if (!getKey()) {
    renderChecklist();
    return;
  }
  try {
    const cfg = await api("/v1/admin/config");
    if (cfg.mesh_enabled && cfg.mesh_bind) markChecklist("bind", true);
  } catch {
    /* ignore */
  }
  try {
    const diag = await api("/v1/admin/diagnostics/join", {
      method: "POST",
      body: { client_time: new Date().toISOString() },
    });
    const byId = Object.fromEntries((diag.checks || []).map((c) => [c.id, c]));
    if (byId.api_key?.ok) markChecklist("key", true);
    if (byId.mesh_bind?.ok) markChecklist("bind", true);
  } catch {
    /* ignore */
  }
  try {
    const jobs = await api("/v1/jobs?limit=1");
    if (Array.isArray(jobs) && jobs.length) markChecklist("submit", true);
  } catch {
    /* ignore */
  }
  if (loadHistory().length) markChecklist("submit", true);
  renderChecklist();
}

function typingInField(el) {
  if (!el) return false;
  const tag = el.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || el.isContentEditable;
}

function initShortcuts() {
  document.addEventListener("keydown", (e) => {
    const meta = e.ctrlKey || e.metaKey;
    const inField = typingInField(document.activeElement);

    // Help: ? (when not typing) or Ctrl+/
    if ((e.key === "?" && !inField && !meta) || (meta && e.key === "/")) {
      e.preventDefault();
      window.location.href = "/ui/help/#keyboard-shortcuts";
      return;
    }

    // Submit job: Ctrl/Cmd+Enter on Jobs panel (or anywhere with job form filled)
    if (meta && e.key === "Enter") {
      const jobsActive = $("#panel-jobs")?.classList.contains("is-active");
      if (jobsActive || document.activeElement?.closest?.("#job-form")) {
        e.preventDefault();
        if (!runtime.online) return;
        $("#job-form")?.requestSubmit();
      }
      return;
    }

    // Refresh mesh: Ctrl/Cmd+Shift+R (avoid browser reload conflict with plain Ctrl+R)
    if (meta && e.shiftKey && (e.key === "R" || e.key === "r")) {
      e.preventDefault();
      switchPanel("nodes");
      refreshMesh();
      return;
    }

    // Copy invite: Ctrl/Cmd+Shift+C
    if (meta && e.shiftKey && (e.key === "C" || e.key === "c")) {
      e.preventDefault();
      const uri = $("#invite-join-uri")?.value;
      if (!uri) {
        switchPanel("nodes");
        showOut($("#mint-out"), "No invite link — mint an invite first.");
        return;
      }
      navigator.clipboard?.writeText(uri).then(
        () => showOut($("#mint-out"), { copied: uri }),
        () => {
          $("#invite-join-uri").select();
        }
      );
    }
  });
}

function schedulePolling() {
  if (runtime.pollTimer) clearInterval(runtime.pollTimer);
  const interval = Math.max(runtime.pollBackoffMs, BASE_POLL_MS);
  runtime.pollTimer = setInterval(async () => {
    if (Date.now() < runtime.throttledUntil) {
      setThrottle(null);
      return;
    }
    try {
      await api("/health", { auth: false });
      setApiOnline(true);
    } catch {
      setApiOnline(false);
      return;
    }
    const active = $(".panel.is-active");
    if (!active || !getKey()) return;
    if (active.id === "panel-monitor") refreshMonitor();
    if (active.id === "panel-nodes") refreshMeshHealth();
    if (active.id === "panel-host") refreshDrain();
    clearThrottleIfIdle();
  }, interval);
}

initThemes();
wireForms();
initShortcuts();
refreshTemplates();
renderChecklist();
refreshHost();
refreshDrain();
loadSupportHint();
refreshChecklistFromDiagnostics();
schedulePolling();
