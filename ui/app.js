(() => {
  "use strict";

  const API = "http://127.0.0.1:8787";
  const STORAGE_KEY = "pull-lessons:monitored";
  const $ = (selector) => document.querySelector(selector);
  const $$ = (selector) => [...document.querySelectorAll(selector)];

  const state = {
    session: null,
    meta: null,
    courses: [],
    filtered: [],
    monitored: new Set(),
    grab: null,
    pollTimer: null,
    autoRefresh: null,
    loginBusy: false,
    backendReady: false,
  };

  /* ---------- 基础工具 ---------- */

  const ESCAPES = { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" };
  const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (ch) => ESCAPES[ch]);
  const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

  function debounce(fn, wait) {
    let timer = null;
    return (...args) => {
      clearTimeout(timer);
      timer = setTimeout(() => fn(...args), wait);
    };
  }

  function toast(message, type = "info", timeout = 4200) {
    const root = $("#toast-root");
    const el = document.createElement("div");
    el.className = `toast ${type}`;
    el.innerHTML = `<span class="t-dot"></span><span>${esc(message)}</span>`;
    root.appendChild(el);
    setTimeout(() => {
      el.classList.add("leaving");
      setTimeout(() => el.remove(), 220);
    }, timeout);
  }

  function setLoginStatus(text, type = "info") {
    const box = $("#login-status");
    box.className = `login-status ${type}`;
    $("#login-status-text").textContent = text;
  }

  function formatTerm(term) {
    if (/^\d{5}$/.test(term || "")) {
      const year = Number(term.slice(0, 4));
      return `${year}-${year + 1} 学年 · 第 ${term.slice(4)} 学期`;
    }
    return term || "—";
  }

  function formatClock(ms) {
    if (!ms) return "--:--:--";
    return new Date(ms).toLocaleTimeString("zh-CN", { hour12: false });
  }

  function formatElapsed(grab) {
    if (!grab || !grab.started_at_ms) return "00:00";
    const end = grab.finished_at_ms || Date.now();
    const total = Math.max(0, Math.floor((end - grab.started_at_ms) / 1000));
    return `${String(Math.floor(total / 60)).padStart(2, "0")}:${String(total % 60).padStart(2, "0")}`;
  }

  function beep() {
    try {
      const Ctx = window.AudioContext || window.webkitAudioContext;
      const ctx = new Ctx();
      [880, 1174.7].forEach((freq, index) => {
        const osc = ctx.createOscillator();
        const gain = ctx.createGain();
        const start = ctx.currentTime + index * 0.16;
        osc.type = "sine";
        osc.frequency.value = freq;
        gain.gain.setValueAtTime(0.0001, start);
        gain.gain.exponentialRampToValueAtTime(0.16, start + 0.02);
        gain.gain.exponentialRampToValueAtTime(0.0001, start + 0.5);
        osc.connect(gain);
        gain.connect(ctx.destination);
        osc.start(start);
        osc.stop(start + 0.55);
      });
      setTimeout(() => ctx.close(), 1600);
    } catch {
      /* 忽略音频不可用 */
    }
  }

  async function api(path, options = {}) {
    let response;
    try {
      response = await fetch(API + path, {
        headers: { "Content-Type": "application/json" },
        ...options,
      });
    } catch (error) {
      throw new Error(`无法连接本地服务：${error.message}`);
    }

    const text = await response.text();
    let body;
    try {
      body = JSON.parse(text);
    } catch {
      throw new Error(`后端响应异常：${text.slice(0, 120)}`);
    }

    if (response.status === 401 || body.code === 401) {
      handleSessionExpired();
      throw new Error(body.msg || "登录已失效");
    }
    if (!response.ok || body.code !== 0) {
      throw new Error(body.msg || `请求失败（HTTP ${response.status}）`);
    }
    return body.data;
  }

  /* ---------- 启动与登录 ---------- */

  async function waitForBackend() {
    for (let attempt = 0; attempt < 80; attempt += 1) {
      try {
        const response = await fetch(`${API}/api/health`, { cache: "no-store" });
        if (response.ok) return;
      } catch {
        /* 继续等待 */
      }
      await sleep(250);
    }
    throw new Error("本地服务未启动");
  }

  async function boot() {
    setLoginStatus("正在连接本地服务…", "busy");
    try {
      await waitForBackend();
      state.backendReady = true;
    } catch {
      setLoginStatus("本地服务未能启动，请重新打开应用。", "error");
      return;
    }

    try {
      const session = await api("/api/auth/status");
      if (session && session.authenticated) {
        await enterApp(session);
        return;
      }
    } catch {
      /* 未登录 */
    }
    showLogin("连接正常，请打开学校登录窗口完成统一身份认证", "info");
  }

  function showLogin(message, type = "info") {
    state.session = null;
    $("#app-screen").classList.add("hidden");
    $("#login-screen").classList.remove("hidden");
    $("#btn-login").disabled = false;
    if (message) setLoginStatus(message, type);
  }

  async function startLogin() {
    if (!window.__TAURI__) {
      setLoginStatus("请使用 Tauri 桌面端运行本应用。", "error");
      return;
    }
    $("#btn-sync").classList.remove("hidden");
    try {
      await window.__TAURI__.core.invoke("start_login");
      setLoginStatus("登录窗口已打开，请完成统一身份认证。登录成功后会自动同步。", "info");
    } catch (error) {
      setLoginStatus(String(error?.message || error), "error");
    }
  }

  async function syncLogin() {
    if (state.loginBusy) return;
    if (!window.__TAURI__) {
      setLoginStatus("请使用 Tauri 桌面端运行本应用。", "error");
      return;
    }

    state.loginBusy = true;
    $("#btn-login").disabled = true;
    $("#btn-sync").disabled = true;
    setLoginStatus("正在读取登录信息并同步到本地服务…", "busy");

    try {
      const session = await window.__TAURI__.core.invoke("login_from_webview");
      await enterApp(session);
      toast("登录成功", "success");
    } catch (error) {
      const message = typeof error === "string" ? error : error?.message || "登录同步失败";
      setLoginStatus(message, "error");
      state.loginBusy = false;
      $("#btn-login").disabled = false;
    } finally {
      $("#btn-sync").disabled = false;
    }
  }

  async function enterApp(session) {
    if (!session || !session.authenticated) {
      showLogin("登录状态无效，请重新登录", "error");
      return;
    }

    state.session = session;
    state.loginBusy = false;
    $("#login-screen").classList.add("hidden");
    $("#app-screen").classList.remove("hidden");
    renderHeader();

    $("#course-list").innerHTML = `<div class="loading-state"><span class="spinner"></span><span>正在加载课程…</span></div>`;

    await Promise.all([loadMeta(), loadCourses({ silent: true }), loadSelectedCount()]);
    renderMonitorQueue();
    renderCourses();
    startAutoRefresh();

    try {
      const grab = await api("/api/grab/status");
      if (grab.running || grab.started_at_ms) {
        state.grab = grab;
        renderGrab();
        renderMonitorQueue();
        if (grab.running) startPolling();
      }
    } catch {
      /* 无在运行任务 */
    }
  }

  function renderHeader() {
    const session = state.session;
    $("#user-name").textContent = session.user.name;
    $("#user-account").textContent = session.user.account;
    $("#user-avatar").textContent = (session.user.name || "?").slice(0, 1);
    $("#term-pill").textContent = formatTerm(session.round.term);
    $("#round-pill").textContent = `轮次 ${session.round.round_code || "—"}`;
  }

  function handleSessionExpired() {
    stopPolling();
    if (!state.session) return;
    state.session = null;
    state.grab = null;
    showLogin("登录已失效，请重新登录", "error");
  }

  async function logout() {
    if (!confirm("确定退出登录吗？正在运行的监听任务会被停止。")) return;
    stopPolling();
    try {
      await api("/api/auth/logout", { method: "POST", body: "{}" });
    } catch {
      /* 即便后端失败也回到登录页 */
    }
    state.session = null;
    state.grab = null;
    showLogin("已退出登录", "info");
  }

  function initTauriEvents() {
    const tauri = window.__TAURI__;
    if (!tauri) return;

    tauri.event
      .listen("login-ready", () => {
        setLoginStatus("检测到登录成功，正在同步会话…", "busy");
        syncLogin();
      })
      .catch(() => {});

    tauri.event
      .listen("login-success", (event) => {
        if (!state.session && event.payload) enterApp(event.payload);
      })
      .catch(() => {});

    tauri.event
      .listen("login-failed", (event) => {
        const message = event.payload?.message || "登录失败";
        setLoginStatus(message, "error");
        $("#btn-sync").classList.remove("hidden");
      })
      .catch(() => {});
  }

  /* ---------- 课程数据 ---------- */

  async function loadMeta() {
    try {
      const meta = await api("/api/meta");
      state.meta = meta;
      const select = $("#category-select");
      const current = select.value;
      select.innerHTML =
        `<option value="">全部大类</option>` +
        (meta.categories || [])
          .map((item) => `<option value="${esc(item.code)}">${esc(item.name)}</option>`)
          .join("");
      if (current) select.value = current;
    } catch (error) {
      if (!String(error.message).includes("登录")) toast(error.message, "error");
    }
  }

  async function loadCourses({ silent = false } = {}) {
    if (!silent) {
      $("#course-list").innerHTML = `<div class="loading-state"><span class="spinner"></span><span>正在刷新课程…</span></div>`;
    }
    try {
      const data = await api("/api/courses");
      state.courses = data.courses || [];
      updateStats(data);
      applyFilters();
      renderMonitorQueue();
    } catch (error) {
      if (!String(error.message).includes("登录")) {
        if (!silent) {
          $("#course-list").innerHTML = `<div class="empty-state"><div class="empty-icon">!</div><p>课程加载失败</p><small>${esc(error.message)}</small></div>`;
        }
        toast(error.message, "error");
      }
    }
  }

  async function loadSelectedCount() {
    try {
      const data = await api("/api/selected");
      const count = (data.courses || []).length;
      $("#btn-selected").textContent = count ? `已选课程 (${count})` : "已选课程";
    } catch {
      /* 忽略 */
    }
  }

  function updateStats(data) {
    const monitored = state.monitored.size;
    if (data) {
      $("#stat-total").textContent = data.total ?? state.courses.length;
      $("#stat-available").textContent = data.available_count ?? 0;
      $("#stat-remaining").textContent = data.total_remaining ?? 0;
    } else {
      $("#stat-total").textContent = state.courses.length;
      $("#stat-available").textContent = state.courses.filter((course) => course.available).length;
      $("#stat-remaining").textContent = state.courses.reduce((sum, course) => sum + course.remaining, 0);
    }
    $("#stat-monitored").textContent = monitored;
  }

  function applyFilters() {
    const keyword = $("#search-input").value.trim().toLowerCase();
    const category = $("#category-select").value;
    const onlyAvailable = $("#only-available").checked;

    state.filtered = state.courses.filter((course) => {
      if (category && course.category_code !== category) return false;
      if (onlyAvailable && !course.available) return false;
      if (!keyword) return true;

      const haystack = [
        course.name,
        course.course_code,
        course.course_id,
        course.department,
        course.category_name,
        course.sub_category_name,
        ...(course.tags || []).map((tag) => tag.name),
        ...(course.classes || []).flatMap((cls) => [
          cls.class_code,
          ...(cls.schedules || []).map((schedule) => schedule.teacher),
        ]),
      ]
        .filter(Boolean)
        .join(" ")
        .toLowerCase();

      return haystack.includes(keyword);
    });

    renderCourses();
    updateStats();
  }

  function renderSlots(schedules) {
    const list = (schedules || []).slice(0, 3);
    const html = list
      .map((schedule) => {
        const time = [
          schedule.weekday,
          schedule.start_period && schedule.end_period
            ? `${schedule.start_period}-${schedule.end_period}节`
            : "",
          schedule.start_week && schedule.end_week
            ? `${schedule.start_week}-${schedule.end_week}周`
            : "",
        ]
          .filter(Boolean)
          .join(" ");
        const extra = [schedule.location, schedule.teacher].filter(Boolean).map(esc).join(" · ");
        return `<span class="slot"><b>${esc(time || "时间待定")}</b>${extra ? ` ${extra}` : ""}</span>`;
      })
      .join("");
    const rest = (schedules || []).length - list.length;
    return html + (rest > 0 ? `<span class="slot more">+${rest} 个时间</span>` : "");
  }

  function renderClasses(course) {
    return (course.classes || [])
      .map((cls) => {
        const slots = (cls.schedules || [])
          .slice(0, 2)
          .map((schedule) =>
            [
              schedule.weekday,
              schedule.start_period && schedule.end_period
                ? `${schedule.start_period}-${schedule.end_period}节`
                : "",
              schedule.location,
            ]
              .filter(Boolean)
              .join(" "),
          )
          .join("；");
        return `<div class="class-item">
          <div class="class-main">
            <strong>${esc(cls.class_name || cls.target || "课堂")} · ${esc(cls.class_code)}</strong>
            <small>${esc(slots || "授课时间待定")}</small>
          </div>
          <div class="class-side">
            <span class="class-capacity">${cls.selected}/${cls.capacity}</span>
            <button class="select-btn" type="button"
              data-select-class="${esc(cls.class_code)}"
              data-course="${esc(course.course_code)}"
              ${cls.available ? "" : "disabled"}>选这个</button>
          </div>
        </div>`;
      })
      .join("");
  }

  function renderCourses() {
    const list = $("#course-list");
    if (!state.filtered.length) {
      list.innerHTML = `<div class="empty-state">
        <div class="empty-icon">∅</div>
        <p>没有符合条件的课程</p>
        <small>试试清空搜索条件，或关闭“只看有余量”</small>
      </div>`;
      return;
    }

    const running = Boolean(state.grab?.running);
    list.innerHTML = state.filtered
      .map((course) => {
        const monitored = state.monitored.has(course.course_code);
        const percent = course.capacity
          ? Math.min(100, Math.round((course.selected / course.capacity) * 100))
          : 0;
        const barClass = course.available ? "ok" : percent >= 85 ? "full" : "warn";
        const firstAvailable = (course.classes || []).find((cls) => cls.available);
        const primaryClass = firstAvailable || (course.classes || [])[0] || { schedules: [] };
        const tags = (course.tags || [])
          .map((tag) => `<span class="tag t${esc(tag.code)}">${esc(tag.name)}</span>`)
          .join("");
        const badge = course.available
          ? `<span class="badge ok">余量 ${course.remaining}</span>`
          : course.remaining > 0
            ? `<span class="badge warn">有余量待开放</span>`
            : `<span class="badge full">已满</span>`;
        const classes = (course.class_count || 1) > 1
          ? `<details class="classes"><summary>共 ${course.class_count} 个课堂</summary><div class="class-list">${renderClasses(course)}</div></details>`
          : "";

        return `<article class="course-card ${course.available ? "is-available" : ""} ${monitored ? "is-monitored" : ""}">
          <div class="course-body">
            <div class="course-title-row">
              <h3 class="course-title">${esc(course.name)}</h3>
              <div class="badges">${badge}</div>
            </div>
            <div class="course-meta">${esc(course.course_id || course.course_code)} · ${esc(course.department)} · ${course.credits} 学分 / ${course.hours} 学时</div>
            ${tags ? `<div class="tags">${tags}</div>` : ""}
            <div class="capacity-row">
              <div class="bar"><i class="${barClass}" style="width:${percent}%"></i></div>
              <span class="capacity-text">已选 <strong>${course.selected}</strong> / 容量 ${course.capacity}</span>
            </div>
            <div class="schedule-row">${renderSlots(primaryClass.schedules)}</div>
            ${classes}
          </div>
          <div class="course-actions">
            <button class="monitor-btn ${monitored ? "active" : ""}" type="button"
              data-monitor="${esc(course.course_code)}" ${running ? "disabled" : ""}>
              ${monitored ? "✓ 监听中" : "加入监听"}
            </button>
            <button class="select-btn" type="button"
              data-select-course="${esc(course.course_code)}"
              ${course.available ? "" : "disabled"}>立即选课</button>
          </div>
        </article>`;
      })
      .join("");
  }

  function toggleMonitor(code, forceRemove = false) {
    if (state.grab?.running) {
      toast("监听运行中，请先停止任务再调整目标", "warn");
      return;
    }
    if (forceRemove || state.monitored.has(code)) state.monitored.delete(code);
    else state.monitored.add(code);
    localStorage.setItem(STORAGE_KEY, JSON.stringify([...state.monitored]));
    updateStats();
    renderCourses();
    renderMonitorQueue();
  }

  /* ---------- 立即选课 ---------- */

  async function selectCourse(courseCode, classCode = null) {
    const course = state.courses.find((item) => item.course_code === courseCode);
    if (!course) return;

    let target = null;
    if (classCode) {
      target = (course.classes || []).find((cls) => cls.class_code === classCode) || null;
    } else {
      target = (course.classes || [])
        .filter((cls) => cls.available)
        .sort((a, b) => b.remaining - a.remaining)[0] || null;
    }

    if (!target) {
      toast("该课程当前没有可选课堂", "warn");
      return;
    }
    if (!confirm(`确定选修《${course.name}》吗？\n课堂：${target.class_code}\n人数：${target.selected}/${target.capacity}`)) {
      return;
    }

    try {
      const data = await api("/api/select", {
        method: "POST",
        body: JSON.stringify({ class_code: target.class_code }),
      });
      toast(data?.upstream_msg || "选课成功", "success");
      if (data?.success) {
        await stopMonitoringSilently();
        showSuccessModal(course.name, { class_code: target.class_code });
        beep();
      }
      loadCourses({ silent: true });
      loadSelectedCount();
    } catch (error) {
      toast(error.message, "error");
    }
  }

  /* ---------- 监听队列 ---------- */

  function renderMonitorQueue() {
    const queue = [...state.monitored];
    const running = Boolean(state.grab?.running);
    $("#monitor-count").textContent = queue.length;
    $("#stat-monitored").textContent = queue.length;

    const container = $("#monitor-queue");
    if (!queue.length) {
      container.innerHTML = `<div class="empty-state small">
        <div class="empty-icon">＋</div>
        <p>还没有监听课程</p>
        <small>在左侧课程卡片上点击“加入监听”</small>
      </div>`;
    } else {
      container.innerHTML = queue
        .map((code, index) => {
          const course = state.courses.find((item) => item.course_code === code);
          const name = course?.name || code;
          const sub = course
            ? `${course.selected}/${course.capacity} · ${course.available ? `余量 ${course.remaining}` : "已满"}`
            : "等待课程数据";
          return `<div class="queue-item">
            <span class="q-index">${index + 1}</span>
            <div class="q-main">
              <div class="q-name" title="${esc(name)}">${esc(name)}</div>
              <div class="q-sub ${course?.available ? "ok" : ""}">${esc(sub)}</div>
            </div>
            <button class="q-remove" type="button" data-remove-monitor="${esc(code)}"
              ${running ? "disabled" : ""} title="移除">×</button>
          </div>`;
        })
        .join("");
    }

    $("#btn-start").disabled = !queue.length || running;
    $("#btn-start").classList.toggle("hidden", running);
    $("#btn-stop").classList.toggle("hidden", !running);
    $("#interval-select").disabled = running;
    $("#monitor-hint").textContent = running
      ? "任务运行中，目标已锁定。抢到任意一门课程后会自动结束。"
      : "监听期间会持续查询课程余量，有余量时立刻提交选课。";
  }

  async function startGrab() {
    const targets = [...state.monitored].map((code) => ({ kind: "course", value: code }));
    if (!targets.length) return;

    $("#btn-start").disabled = true;
    try {
      const data = await api("/api/grab/start", {
        method: "POST",
        body: JSON.stringify({
          targets,
          interval_ms: Number($("#interval-select").value),
          max_attempts: 0,
          check_capacity_first: true,
        }),
      });
      state.grab = data;
      renderGrab();
      renderMonitorQueue();
      startPolling();
      toast(`已开始监听 ${targets.length} 门课程`, "success");
    } catch (error) {
      toast(error.message, "error");
      $("#btn-start").disabled = false;
      try {
        const grab = await api("/api/grab/status");
        if (grab.running) {
          state.grab = grab;
          renderGrab();
          renderMonitorQueue();
          startPolling();
        }
      } catch {
        /* 忽略 */
      }
    }
  }

  async function stopGrab() {
    try {
      const data = await api("/api/grab/stop", { method: "POST", body: "{}" });
      state.grab = data;
      stopPolling();
      renderGrab();
      renderMonitorQueue();
      toast("已停止监听", "info");
    } catch (error) {
      toast(error.message, "error");
    }
  }

  // 手动选课成功时，立即结束正在运行的监听任务，避免继续抢其他课程。
  async function stopMonitoringSilently() {
    if (!state.grab?.running) return;
    stopPolling();
    try {
      state.grab = await api("/api/grab/stop", { method: "POST", body: "{}" });
    } catch {
      /* 即使停止接口失败也不再轮询 */
    }
    if (state.grab) {
      state.grab.running = false;
      state.grab.success = true;
      state.grab.stopped_reason = "success";
      state.grab.last_message = "选课成功，监听任务已结束";
    }
    renderGrab();
    renderMonitorQueue();
  }

  function startPolling() {
    stopPolling();
    const tick = async () => {
      try {
        const data = await api("/api/grab/status");
        state.grab = data;
        renderGrab();
        renderMonitorQueue();

        if (data.success) {
          stopPolling();
          onGrabSuccess(data);
          return;
        }
        if (!data.running) {
          stopPolling();
          if (data.stopped_reason === "session_expired") {
            toast("登录已失效，请重新登录", "error");
          } else if (data.stopped_reason === "rejected") {
            toast(data.last_message || "学校拒绝了选课请求", "error");
          }
          return;
        }
        state.pollTimer = setTimeout(tick, 1000);
      } catch (error) {
        if (String(error.message).includes("登录")) return;
        state.pollTimer = setTimeout(tick, 2200);
      }
    };
    state.pollTimer = setTimeout(tick, 300);
  }

  function stopPolling() {
    if (state.pollTimer) {
      clearTimeout(state.pollTimer);
      state.pollTimer = null;
    }
  }

  function onGrabSuccess(data) {
    const selection = data.selection || {};
    const course = state.courses.find((item) => item.course_code === selection.course_code);
    const name = selection.course_name || course?.name || selection.class_code || "目标课程";
    beep();
    showSuccessModal(name, selection);
    toast("抢课成功，任务已完成", "success", 6000);
    loadCourses({ silent: true });
    loadSelectedCount();
  }

  const EVENT_LABELS = {
    start: "启动",
    attempt: "轮询",
    wait: "等待",
    found: "发现",
    select: "提交",
    success: "成功",
    error: "错误",
    stop: "结束",
  };

  function renderGrab() {
    const grab = state.grab;
    const box = $("#grab-status");
    if (!grab || (!grab.running && !grab.started_at_ms && !(grab.targets || []).length)) {
      box.classList.add("hidden");
      return;
    }
    box.classList.remove("hidden");

    const stateEl = $("#grab-state");
    let label = "已停止";
    let cls = "stopped";
    if (grab.running) {
      label = "监听中";
      cls = "running";
    } else if (grab.success) {
      label = "已完成";
      cls = "success";
    } else if (grab.stopped_reason === "session_expired" || grab.stopped_reason === "rejected") {
      label = "已中止";
      cls = "error";
    } else if (grab.stopped_reason === "stopped_by_user") {
      label = "已停止";
      cls = "stopped";
    }
    stateEl.className = `grab-state ${cls}`;
    stateEl.textContent = label;

    $("#grab-attempts").textContent = grab.attempts ?? 0;
    $("#grab-calls").textContent = grab.select_calls ?? 0;
    $("#grab-result").textContent = grab.success ? "成功" : grab.running ? "—" : grab.last_code == null ? "—" : String(grab.last_code);
    $("#grab-message").textContent = grab.last_message || (grab.running ? "正在轮询课程余量…" : "—");
    $("#grab-elapsed").textContent = formatElapsed(grab);

    const logs = grab.logs || [];
    $("#grab-logs").innerHTML = logs.length
      ? logs
          .slice(-40)
          .reverse()
          .map(
            (log) => `<div class="log-item">
              <span class="log-time">${formatClock(log.at_ms)}</span>
              <span class="log-event ${esc(log.event)}">${EVENT_LABELS[log.event] || esc(log.event)}</span>
              <span class="log-msg">${esc(log.message)}</span>
            </div>`,
          )
          .join("")
      : `<div class="empty-logs">暂无日志</div>`;
  }

  /* ---------- 已选课程 / 弹窗 ---------- */

  function openModal(html) {
    const root = $("#modal-root");
    root.innerHTML = `<div class="modal">${html}</div>`;
    root.classList.remove("hidden");
  }

  function closeModal() {
    const root = $("#modal-root");
    root.classList.add("hidden");
    root.innerHTML = "";
  }

  function showSuccessModal(courseName, selection) {
    openModal(`
      <div class="success-hero">
        <div class="success-icon">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round">
            <path d="m5 12.5 4.5 4.5L19 7.5"/>
          </svg>
        </div>
        <h2 class="success-title">抢课成功</h2>
        <p class="success-text">已为你完成选课，监听任务已自动结束，目标课程已经加入课表。</p>
        <div class="success-course">${esc(courseName)}
          <small>课堂 ${esc(selection?.class_code || "—")}</small>
        </div>
      </div>
      <div class="modal-actions">
        <button class="btn ghost" type="button" data-modal-close>知道了</button>
        <button class="btn primary" type="button" data-show-selected>查看已选课程</button>
      </div>
    `);
  }

  async function showSelected() {
    try {
      const data = await api("/api/selected");
      const courses = data.courses || [];
      const body = courses.length
        ? `<div class="selected-list">${courses
            .map((course) => {
              const schedule = (course.schedules || [])
                .slice(0, 2)
                .map((item) =>
                  [item.weekday, item.start_period && item.end_period ? `${item.start_period}-${item.end_period}节` : "", item.location]
                    .filter(Boolean)
                    .join(" "),
                )
                .filter(Boolean)
                .join("；");
              return `<div class="selected-item">
                <div class="s-main">
                  <strong>${esc(course.name)}</strong>
                  <small>${esc(course.class_code)}${schedule ? ` · ${esc(schedule)}` : ""}</small>
                </div>
                ${course.can_drop ? `<button class="btn ghost danger" type="button" data-drop-class="${esc(course.class_code)}">退选</button>` : ""}
              </div>`;
            })
            .join("")}</div>`
        : `<div class="empty-state small"><div class="empty-icon">○</div><p>还没有已选课程</p><small>监听成功后课程会出现在这里</small></div>`;

      openModal(`
        <div class="modal-head">
          <div>
            <h2>已选课程</h2>
            <p class="modal-sub">${esc(data.term || "")} · 共 ${courses.length} 门</p>
          </div>
          <button class="modal-close" type="button" data-modal-close>×</button>
        </div>
        <div class="modal-body">${body}</div>
      `);
    } catch (error) {
      toast(error.message, "error");
    }
  }

  async function dropCourse(classCode) {
    if (!confirm("确定退选该课堂吗？")) return;
    try {
      await api("/api/drop", {
        method: "POST",
        body: JSON.stringify({ class_code: classCode }),
      });
      toast("退选成功", "success");
      closeModal();
      showSelected();
      loadCourses({ silent: true });
      loadSelectedCount();
    } catch (error) {
      toast(error.message, "error");
    }
  }

  /* ---------- 自动刷新 ---------- */

  function startAutoRefresh() {
    if (state.autoRefresh) clearInterval(state.autoRefresh);
    state.autoRefresh = setInterval(() => {
      if (document.hidden || !state.session || state.grab?.running) return;
      loadCourses({ silent: true });
    }, 20000);
  }

  /* ---------- 事件绑定 ---------- */

  function bindEvents() {
    $("#btn-login").addEventListener("click", startLogin);
    $("#btn-sync").addEventListener("click", syncLogin);
    $("#btn-logout").addEventListener("click", logout);
    $("#btn-selected").addEventListener("click", showSelected);
    $("#btn-refresh").addEventListener("click", async () => {
      const button = $("#btn-refresh");
      button.disabled = true;
      await loadCourses();
      button.disabled = false;
      toast("课程列表已刷新", "info", 2200);
    });

    $("#search-input").addEventListener("input", debounce(applyFilters, 150));
    $("#category-select").addEventListener("change", applyFilters);
    $("#only-available").addEventListener("change", applyFilters);

    $("#course-list").addEventListener("click", (event) => {
      const monitorButton = event.target.closest("[data-monitor]");
      if (monitorButton) {
        toggleMonitor(monitorButton.dataset.monitor);
        return;
      }
      const classButton = event.target.closest("[data-select-class]");
      if (classButton) {
        selectCourse(classButton.dataset.course, classButton.dataset.selectClass);
        return;
      }
      const selectButton = event.target.closest("[data-select-course]");
      if (selectButton) {
        selectCourse(selectButton.dataset.selectCourse);
      }
    });

    $("#monitor-queue").addEventListener("click", (event) => {
      const remove = event.target.closest("[data-remove-monitor]");
      if (remove) toggleMonitor(remove.dataset.removeMonitor, true);
    });

    $("#btn-start").addEventListener("click", startGrab);
    $("#btn-stop").addEventListener("click", stopGrab);

    $("#modal-root").addEventListener("click", (event) => {
      if (event.target.id === "modal-root" || event.target.closest("[data-modal-close]")) {
        closeModal();
        return;
      }
      if (event.target.closest("[data-show-selected]")) {
        closeModal();
        showSelected();
        return;
      }
      const drop = event.target.closest("[data-drop-class]");
      if (drop) dropCourse(drop.dataset.dropClass);
    });

    document.addEventListener("keydown", (event) => {
      if (event.key === "Escape") closeModal();
    });

    window.addEventListener("beforeunload", () => {
      stopPolling();
      if (state.autoRefresh) clearInterval(state.autoRefresh);
    });
  }

  function restoreMonitored() {
    try {
      const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) || "[]");
      if (Array.isArray(saved)) state.monitored = new Set(saved.filter(Boolean));
    } catch {
      state.monitored = new Set();
    }
  }

  function init() {
    restoreMonitored();
    bindEvents();
    initTauriEvents();
    boot();
  }

  document.addEventListener("DOMContentLoaded", init);
})();
