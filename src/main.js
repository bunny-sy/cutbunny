const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

// 색 기준(사용자 설정, localStorage 저장). green=목표(초록상한), yellow=노랑상한, gray=잔컷상한
const DEFAULT_TH = { gray: 1.2, green: 3.0, yellow: 4.0 };
let TH = loadTH();
function loadTH() {
  try {
    const s = JSON.parse(localStorage.getItem("cutcheck_th"));
    if (s && s.green > 0) return { gray: +s.gray || 0, green: +s.green, yellow: +s.yellow };
  } catch (_) {}
  return { ...DEFAULT_TH };
}
function saveTH() {
  localStorage.setItem("cutcheck_th", JSON.stringify(TH));
}
// 길이별 신호등 색: 잔컷(회색) / 초록 / 노랑 / 빨강
function lenClass(dur) {
  if (dur < TH.gray) return "tiny";
  if (dur > TH.yellow) return "over";
  if (dur > TH.green) return "warn";
  return "good";
}

// 썸네일: 길이 표시를 절대 막지 않도록 백그라운드에서 동시 3개까지만 처리
const thumbCache = new Map(); // key -> data URI (성공)
const thumbTried = new Set(); // 요청했던 key (재요청 방지)
const thumbQueue = [];
let thumbActive = 0;
const THUMB_MAX = 5;

function pumpThumbs() {
  while (thumbActive < THUMB_MAX && thumbQueue.length) {
    const job = thumbQueue.shift();
    thumbActive++;
    job().finally(() => {
      thumbActive--;
      pumpThumbs();
    });
  }
}

function loadThumb(imgId, video, src) {
  if (!video) return;
  const key = video + "|" + src.toFixed(2);
  // 이미 성공 캐시면 즉시 표시
  const cached = thumbCache.get(key);
  if (cached) {
    const img = document.getElementById(imgId);
    if (img) img.src = cached;
    return;
  }
  if (thumbTried.has(key)) return; // 이미 시도함 — 성공 시 다음 렌더에서 채워짐
  thumbTried.add(key);
  thumbQueue.push(async () => {
    try {
      const uri = await invoke("thumbnail", { video, srcSec: src });
      thumbCache.set(key, uri);
      const img = document.getElementById(imgId);
      if (img) img.src = uri;
    } catch (_) {
      thumbTried.delete(key); // 실패는 다음에 재시도 허용
    }
  });
  pumpThumbs();
}

let curProject = null;
let followLatest = true;
let lastMod = 0;
let lastDur = [];

function fmt(s) {
  if (s >= 60) return Math.floor(s / 60) + "분 " + (s % 60).toFixed(1) + "초";
  return s.toFixed(2) + "초";
}

// 프로젝트를 못 찾았을 때: 안내 + 폴더 직접 선택 UI 표시
async function showNoProject() {
  $("projSel").style.display = "none";
  $("noProj").style.display = "block";
  $("status").textContent = "캡컷 프로젝트를 찾는 중...";
  try {
    $("npDiag").textContent = await invoke("diag");
  } catch (e) {
    $("npDiag").textContent = String(e);
  }
}

function hideNoProject() {
  $("projSel").style.display = "";
  $("noProj").style.display = "none";
}

async function pickFolder() {
  const msg = $("npMsg");
  try {
    msg.className = "npMsg";
    msg.textContent = "";
    await invoke("pick_root"); // 선택창은 러스트가 띄움
    msg.className = "npMsg ok";
    msg.textContent = "✅ 찾았어요! 불러오는 중...";
    followLatest = true;
    lastMod = 0;
    await refreshProjects();
    await poll();
  } catch (e) {
    msg.className = "npMsg";
    msg.textContent = "⚠ " + (e && e.message ? e.message : e);
  }
}

async function refreshProjects() {
  const projs = await invoke("list_projects");
  if (!projs.length) {
    await showNoProject();
    return;
  }
  hideNoProject();
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

    const barBase = TH.green || 3.0; // 막대 기준 = 목표 길이
    // 진짜 컷(≤10초)과 미편집 덩어리(>10초) 분리
    const real = [];
    let rawSum = 0;
    cuts.forEach((c, i) => {
      if (c.dur > 10) rawSum += c.dur;
      else real.push({ c, i });
    });
    const total = cuts.reduce((s, c) => s + c.dur, 0);
    const realTotal = real.reduce((s, x) => s + x.c.dur, 0);
    $("stCuts").textContent = real.length;
    $("stTotal").textContent = fmt(total);
    $("stAvg").textContent = real.length
      ? (realTotal / real.length).toFixed(2) + "초"
      : "-";

    // 미편집 덩어리 → 하단 배너로
    if (rawSum > 0) {
      $("rawBanner").style.display = "flex";
      $("rawAmount").textContent = fmt(rawSum);
    } else {
      $("rawBanner").style.display = "none";
    }

    const tbody = $("rows");
    tbody.innerHTML = "";
    let firstChanged = null;
    // 역순 렌더 — 최신(마지막) 컷이 맨 위로
    for (let k = real.length - 1; k >= 0; k--) {
      const { c, i } = real[k];
      const tr = document.createElement("tr");
      const changed =
        lastDur.length && Math.abs((lastDur[i] ?? -1) - c.dur) > 0.001;
      if (changed) {
        tr.className = "flash";
        if (firstChanged === null) firstChanged = tr;
      }
      const thumbId = `th${i}`;
      const w = Math.min(100, Math.round((c.dur / barBase) * 100));
      tr.innerHTML =
        `<td class="n">${i + 1}</td>` +
        `<td class="thumb"><img id="${thumbId}" alt=""></td>` +
        `<td class="len ${lenClass(c.dur)}">` +
        `<span class="bar" style="width:${w}%"></span>` +
        `<span class="lentext">${c.dur.toFixed(2)}초</span>` +
        `<span class="starttext">${fmt(c.start)}</span>` +
        `</td>`;
      tbody.appendChild(tr);
      loadThumb(thumbId, c.video, c.src);
    }
    if (firstChanged)
      firstChanged.scrollIntoView({ block: "center", behavior: "smooth" });
    lastDur = cuts.map((c) => c.dur);
    $("status").textContent =
      "버니가 과제 감시중 👀 " + new Date().toLocaleTimeString();
  } catch (e) {
    $("status").textContent = "읽는 중... (" + e + ")";
  }
}

// ---- 디하클 닉네임 게이트 + 사용 기록 (주소는 러스트 내부에만 존재) ----
const CHEERS = [
  "숙제하러 오셨네요 👀",
  "오늘도 컷 자르러 오셨군요, 멋져요 🐰",
  "3초 컷 가봅시다! 화이팅이에요 ✂",
  "버니가 지켜보고 있어요... 힘내세요! 👀",
  "오늘의 목표, 초록불 가득 채워봐요 🟢",
  "숙제 안 하고 어디 가시나 했어요 😆",
  "컷 리듬 살려서 멋지게 만들어봐요 🎬",
  "오셨으니 하나라도 자르고 가요 🥕",
  "오늘도 성실하시네요, 응원할게요 💪",
  "좋은 컷 만들 준비 되셨나요? ✨",
];

// 닉네임 정규화: 앞뒤 공백 제거 + 한글 유니코드 통일(NFC) + BOM 제거
function norm(s) {
  return (s || "").replace(/﻿/g, "").trim().normalize("NFC");
}

async function fetchWhitelist() {
  try {
    const list = await invoke("wl_fetch"); // 주소는 러스트 내부에만 존재
    return new Set(list.map((n) => norm(n)).filter(Boolean));
  } catch (_) {
    return null;
  }
}

function logUsage(nick, ver) {
  try {
    invoke("log_use", { nick, ver: ver || "" }); // 기록 전송도 러스트가 처리
  } catch (_) {}
}

async function startApp(nick) {
  $("gate").style.display = "none";
  $("app").style.display = "flex";
  $("cheer").textContent = CHEERS[Math.floor(Math.random() * CHEERS.length)];

  const ver = await window.__TAURI__.app.getVersion();
  if (nick) logUsage(nick, ver);
  $("ver").textContent = "v" + ver + " · " + (nick || "");
  $("pickBtn").addEventListener("click", pickFolder);

  $("projSel").addEventListener("change", (e) => {
    curProject = e.target.value;
    followLatest = false;
    lastMod = 0;
    lastDur = [];
    poll();
  });

  setupSettings();
  renderLegend();

  await refreshProjects();
  await poll();
  setInterval(refreshProjects, 3000);
  setInterval(poll, 1000);
}

function fmtSec(v) {
  return (Math.round(v * 10) / 10).toString().replace(/\.0$/, "") + "초";
}
function renderLegend() {
  $("lgGray").textContent = "잔컷 " + fmtSec(TH.gray) + "↓";
  $("lgGreen").textContent = "~" + fmtSec(TH.green);
  $("lgYellow").textContent = fmtSec(TH.green) + "~" + fmtSec(TH.yellow);
  $("lgRed").textContent = fmtSec(TH.yellow) + "↑";
}
function applyTH() {
  saveTH();
  renderLegend();
  lastMod = 0; // 다음 폴에서 강제 재렌더
  lastDur = [];
  poll();
}
function setupSettings() {
  const panel = $("settings");
  $("gearBtn").addEventListener("click", () => {
    panel.style.display = panel.style.display === "none" ? "block" : "none";
  });
  const tabS = $("tabSimple"), tabC = $("tabCustom");
  const paneS = $("paneSimple"), paneC = $("paneCustom");
  tabS.addEventListener("click", () => {
    tabS.classList.add("active"); tabC.classList.remove("active");
    paneS.style.display = "block"; paneC.style.display = "none";
  });
  tabC.addEventListener("click", () => {
    tabC.classList.add("active"); tabS.classList.remove("active");
    paneC.style.display = "block"; paneS.style.display = "none";
    fillCustom();
  });
  // 간단: 슬라이더(목표) → green=목표, yellow=목표+1, gray 유지
  const sl = $("tgtSlider");
  sl.value = TH.green;
  $("tgtVal").textContent = fmtSec(TH.green);
  sl.addEventListener("input", () => {
    const t = parseFloat(sl.value);
    $("tgtVal").textContent = fmtSec(t);
    TH.green = t; TH.yellow = t + 1;
    applyTH();
  });
  // 직접: 각 경계 입력
  fillCustom();
  ["cGray", "cGreen", "cYellow"].forEach((id) =>
    $(id).addEventListener("change", () => {
      const g = parseFloat($("cGray").value);
      const gr = parseFloat($("cGreen").value);
      const y = parseFloat($("cYellow").value);
      if (!(gr > 0) || !(y > gr)) return; // 유효성
      TH.gray = g >= 0 ? g : 0;
      TH.green = gr;
      TH.yellow = y;
      sl.value = Math.min(5, Math.max(1.5, gr));
      $("tgtVal").textContent = fmtSec(gr);
      applyTH();
    })
  );
}
function fillCustom() {
  $("cGray").value = TH.gray;
  $("cGreen").value = TH.green;
  $("cYellow").value = TH.yellow;
}

async function gateCheck() {
  const saved = localStorage.getItem("dihacl_nick");
  if (saved) {
    startApp(saved);
    // 백그라운드 재검증 — 명단에서 빠지면 다음 실행부터 잠금
    fetchWhitelist().then((wl) => {
      if (wl && !wl.has(saved)) localStorage.removeItem("dihacl_nick");
    });
    return;
  }
  $("gate").style.display = "flex";
  $("nickInput").focus();
}

async function trySubmit() {
  const nick = norm($("nickInput").value);
  if (!nick) return;
  $("gateMsg").textContent = "확인 중...";
  const wl = await fetchWhitelist();
  if (wl === null) {
    $("gateMsg").textContent = "인터넷 연결을 확인해주세요.";
    return;
  }
  if (!wl.has(nick)) {
    $("gateMsg").textContent = "등록되지 않은 닉네임이에요. 디하클 닉네임을 정확히 입력해주세요.";
    return;
  }
  localStorage.setItem("dihacl_nick", nick);
  startApp(nick);
}

window.addEventListener("DOMContentLoaded", () => {
  $("nickBtn").addEventListener("click", trySubmit);
  $("nickInput").addEventListener("keydown", (e) => {
    if (e.key === "Enter") trySubmit();
  });
  gateCheck();
});
