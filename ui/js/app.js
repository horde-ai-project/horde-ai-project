const STORAGE_KEY = "horde.operator.apiKey";
const HISTORY_KEY = "horde.operator.jobHistory";
const THEME_KEY = "horde.operator.theme";

const THEMES = [
  { id: "slate-teal", label: "Slate teal (default)" },
  { id: "coffee", label: "Coffee" },
  { id: "business-blue", label: "Business blue" },
  { id: "forest", label: "Forest" },
  { id: "graphite", label: "Graphite" },
  { id: "sandstone", label: "Sandstone" },
  { id: "ocean", label: "Ocean" },
  { id: "ink", label: "Ink" },
  { id: "rosewood", label: "Rosewood" },
  { id: "amber-field", label: "Amber field" },
];

const $ = (sel, root = document) => root.querySelector(sel);
const $$ = (sel, root = document) => [...root.querySelectorAll(sel)];

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

async function api(path, { method = "GET", body, auth = true } = {}) {
  const headers = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (auth) {
    const key = getKey();
    if (!key) throw new Error("Set and save an API key first (from the horde startup log).");
    headers.Authorization = `Bearer ${key}`;
  }
  const res = await fetch(`${apiBase()}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  let data = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    data = text;
  }
  if (!res.ok) {
    const msg = data && data.error ? data.error : `${res.status} ${res.statusText}`;
    throw new Error(msg);
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

function applyTheme(id) {
  const theme = THEMES.find((t) => t.id === id) || THEMES[0];
  const link = $("#theme-sheet");
  link.href = `/ui/css/themes/${theme.id}.css`;
  document.documentElement.dataset.theme = theme.id;
  localStorage.setItem(THEME_KEY, theme.id);
  const select = $("#theme-select");
  if (select) select.value = theme.id;
}

function initThemes() {
  const select = $("#theme-select");
  select.innerHTML = THEMES.map((t) => `<option value="${t.id}">${t.label}</option>`).join("");
  const saved = localStorage.getItem(THEME_KEY) || "slate-teal";
  applyTheme(saved);
  select.addEventListener("change", () => applyTheme(select.value));
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
  if (name === "nodes") refreshMesh();
  if (name === "jobs") refreshJobsTable();
  if (name === "host") refreshHost();
}

/* ---- Host ---- */

async function refreshHost() {
  try {
    const health = await api("/health", { auth: false });
    $("#host-health").textContent = health.status || "ok";
    setConn("ok", "online");
  } catch (err) {
    $("#host-health").textContent = "unreachable";
    setConn("bad", "offline");
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
    refreshJobsTable();
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
    refreshHost();
    loadSupportHint();
  });

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
      refreshHost();
      refreshMesh();
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

  $("#mint-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const body = {
      purpose: $("#mint-purpose").value,
      ttl_secs: Number($("#mint-ttl").value) || 3600,
    };
    const note = $("#mint-note").value.trim();
    if (note) body.note = note;
    try {
      const out = await api("/v1/admin/invites", { method: "POST", body });
      showOut($("#mint-out"), out);
    } catch (err) {
      showOut($("#mint-out"), String(err.message || err));
    }
  });

  $("#join-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    try {
      const out = await api("/v1/admin/mesh/join", {
        method: "POST",
        body: {
          peer_addr: $("#join-addr").value.trim(),
          token: $("#join-token").value.trim(),
        },
      });
      showOut($("#join-out"), out);
      refreshMesh();
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

  $("#job-form").addEventListener("submit", async (e) => {
    e.preventDefault();
    const kind = $("#job-kind").value;
    const body = {
      fingerprint: {
        model_id: $("#job-model").value.trim(),
        revision_hash: $("#job-rev").value.trim(),
        precision: $("#job-precision").value.trim() || "fp16",
      },
      kind,
      payload: $("#job-payload").value,
      routing: $("#job-routing").value,
    };
    const tok = $("#job-tok").value.trim();
    if (tok) body.fingerprint.tokenizer_revision = tok;
    try {
      const out = await api("/v1/jobs", { method: "POST", body });
      showOut($("#job-create-out"), out);
      if (out.id) {
        $("#track-id").value = out.id;
        rememberJob(out.id);
        refreshJobsTable();
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
      showOut($("#job-track-out"), await api(`/v1/jobs/${encodeURIComponent(id)}/result`));
    } catch (err) {
      showOut($("#job-track-out"), String(err.message || err));
    }
  });

  $("#jobs-refresh").addEventListener("click", refreshJobsTable);
  $("#jobs-clear-local").addEventListener("click", () => {
    localStorage.removeItem(HISTORY_KEY);
    refreshJobsTable();
  });

  $("#mon-refresh").addEventListener("click", refreshMonitor);

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

function startPolling() {
  setInterval(() => {
    const active = $(".panel.is-active");
    if (!active || !getKey()) return;
    if (active.id === "panel-monitor") refreshMonitor();
    if (active.id === "panel-host") {
      api("/health", { auth: false })
        .then(() => setConn("ok", "online"))
        .catch(() => setConn("bad", "offline"));
    }
  }, 5000);
}

initThemes();
wireForms();
refreshHost();
loadSupportHint();
startPolling();
