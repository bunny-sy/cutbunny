const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);

// 길이별 신호등 색: ~3초 초록 / 3~4초 노랑 / 4초↑ 빨강
function lenClass(dur) {
  if (dur > 4) return "over";
  if (dur > 3) return "warn";
  return "good";
}

const thumbCache = new Map();
async function loadThumb(imgId, video, src) {
  if (!video) return;
  const key = video + "|" + src.toFixed(2);
  try {
    let uri = thumbCache.get(key);
    if (!uri) {
      uri = await invoke("thumbnail", { video, srcSec: src });
      thumbCache.set(key, uri);
    }
    const img = document.getElementById(imgId);
    if (img) img.src = uri;
  } catch (_) {}
}

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

    const barBase = 3.0; // 막대 기준 = 3초 목표
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

// ---- 디하클 닉네임 게이트 + 사용 기록 ----
const WHITELIST_URL = "https://docs.google.com/spreadsheets/d/e/2PACX-1vRuXVUWXO18F5LEvwBQ3ltKIRd-43PIa6LNOcWmy9i10yQJyH9NaUmi_CwEgd24Mb0la_B5YYIRJJpz/pub?output=csv";
const LOG_FORM_URL = "https://docs.google.com/forms/d/e/1FAIpQLSfk364EN10WbNalCiasXUe1t2fslghaZtmB-QxMIyks9OOBEQ/formResponse";
const LOG_ENTRY = "entry.2041288337"; // 폼 닉네임 필드 id

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
    const r = await fetch(WHITELIST_URL + "&t=" + Date.now());
    let txt = await r.text();
    txt = txt.replace(/^﻿/, ""); // 구글 CSV 첫 줄 BOM 제거
    return new Set(
      txt
        .split(/\r?\n/)
        .map((l) => norm(l.split(",")[0]))
        .filter(Boolean)
    );
  } catch (_) {
    return null;
  }
}

function logUsage(nick) {
  try {
    const fd = new FormData();
    fd.append(LOG_ENTRY, nick);
    fetch(LOG_FORM_URL, { method: "POST", mode: "no-cors", body: fd });
  } catch (_) {}
}

async function startApp(nick) {
  $("gate").style.display = "none";
  $("app").style.display = "flex";
  $("cheer").textContent = CHEERS[Math.floor(Math.random() * CHEERS.length)];
  if (nick) logUsage(nick);

  const ver = await window.__TAURI__.app.getVersion();
  $("ver").textContent = "v" + ver + " · " + (nick || "");
  const root = await invoke("get_root");
  if (!root) $("status").textContent = "⚠ 캡컷 폴더를 못 찾았어요 — 캡컷 설치 확인!";

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
