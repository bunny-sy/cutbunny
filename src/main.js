const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

let curProject = null;
let followLatest = true;
let lastMod = 0;
let lastDur = [];

function fmt(s) {
  if (s >= 60) return Math.floor(s / 60) + "분 " + (s % 60).toFixed(1) + "초";
  return s.toFixed(2) + "초";
}

async function refreshProjects() {
  const projs = await invoke("list_projects");
  if (!projs.length) return;
  const sel = $("projSel");
  const names = projs.map((p) => p.name);
  if (sel.dataset.names !== names.join("|")) {
    sel.innerHTML = names
      .map((n) => `<option value="${n.replace(/"/g, "&quot;")}">${n}</option>`)
      .join("");
    sel.dataset.names = names.join("|");
  }
  if (followLatest) curProject = projs[0].name;
  sel.value = curProject;
}

async function poll() {
  if (!curProject) return;
  try {
    const r = await invoke("read_cuts", { project: curProject });
    if (r.mtime === lastMod) return;
    lastMod = r.mtime;
    const cuts = r.cuts;

    const total = cuts.reduce((s, c) => s + c.dur, 0);
    $("stCuts").textContent = cuts.length;
    $("stTotal").textContent = fmt(total);
    $("stAvg").textContent = cuts.length
      ? (total / cuts.length).toFixed(2) + "초"
      : "-";

    const tbody = $("rows");
    tbody.innerHTML = "";
    let firstChanged = null;
    cuts.forEach((c, i) => {
      const tr = document.createElement("tr");
      const changed =
        lastDur.length && Math.abs((lastDur[i] ?? -1) - c.dur) > 0.001;
      if (changed) {
        tr.className = "flash";
        if (firstChanged === null) firstChanged = tr;
      }
      tr.innerHTML =
        `<td class="n">${i + 1}</td><td>${fmt(c.start)}</td>` +
        `<td class="len">${c.dur.toFixed(2)}초</td>`;
      tbody.appendChild(tr);
    });
    if (firstChanged)
      firstChanged.scrollIntoView({ block: "center", behavior: "smooth" });
    lastDur = cuts.map((c) => c.dur);
    $("status").textContent =
      "감시 중 🐰 " + new Date().toLocaleTimeString();
  } catch (e) {
    $("status").textContent = "읽는 중... (" + e + ")";
  }
}

window.addEventListener("DOMContentLoaded", async () => {
  const root = await invoke("get_root");
  $("rootInfo").textContent = root || "⚠ 캡컷 폴더를 못 찾았어요 — 캡컷 설치 확인!";

  $("projSel").addEventListener("change", (e) => {
    curProject = e.target.value;
    followLatest = false;
    lastMod = 0;
    lastDur = [];
    poll();
  });

  await refreshProjects();
  await poll();
  setInterval(refreshProjects, 3000);
  setInterval(poll, 1000);
});
