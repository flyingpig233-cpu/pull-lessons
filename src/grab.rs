use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use crate::error::{AppError, ClientError, now_ms};
use crate::models::{RawClassRoom, class_is_available};
use crate::state::AppState;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "lowercase")]
pub enum GrabTarget {
    Class(String),
    Course(String),
}

impl GrabTarget {
    pub fn describe(&self) -> String {
        match self {
            GrabTarget::Class(code) => format!("课堂 {code}"),
            GrabTarget::Course(code) => format!("课程 {code}"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct GrabSelection {
    pub class_code: String,
    pub course_code: String,
    pub course_name: String,
}

#[derive(Clone, Debug)]
pub struct GrabConfig {
    pub interval_ms: u64,
    pub max_attempts: u64,
    pub check_capacity_first: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct GrabLog {
    pub at_ms: u64,
    pub event: String,
    pub message: String,
    pub code: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct GrabStatus {
    pub running: bool,
    pub targets: Vec<GrabTarget>,
    pub attempts: u64,
    pub select_calls: u64,
    pub started_at_ms: u64,
    pub finished_at_ms: Option<u64>,
    pub success: bool,
    pub selection: Option<GrabSelection>,
    pub last_code: Option<i64>,
    pub last_message: String,
    pub stopped_reason: Option<String>,
    pub logs: Vec<GrabLog>,
}

impl GrabStatus {
    fn idle() -> Self {
        Self {
            running: false,
            targets: Vec::new(),
            attempts: 0,
            select_calls: 0,
            started_at_ms: 0,
            finished_at_ms: None,
            success: false,
            selection: None,
            last_code: None,
            last_message: String::new(),
            stopped_reason: None,
            logs: Vec::new(),
        }
    }

    fn starting(targets: &[GrabTarget], cfg: &GrabConfig) -> Self {
        let now = now_ms();
        let names = targets
            .iter()
            .map(GrabTarget::describe)
            .collect::<Vec<_>>()
            .join("、");
        Self {
            running: true,
            targets: targets.to_vec(),
            attempts: 0,
            select_calls: 0,
            started_at_ms: now,
            finished_at_ms: None,
            success: false,
            selection: None,
            last_code: None,
            last_message: String::new(),
            stopped_reason: None,
            logs: vec![GrabLog {
                at_ms: now,
                event: "start".to_string(),
                message: format!(
                    "开始监听 {} 个目标（{}），间隔 {}ms，最多 {} 次",
                    targets.len(),
                    names,
                    cfg.interval_ms,
                    if cfg.max_attempts == 0 {
                        "不限".to_string()
                    } else {
                        cfg.max_attempts.to_string()
                    }
                ),
                code: None,
            }],
        }
    }
}

struct GrabHandle {
    status: Arc<Mutex<GrabStatus>>,
    stop: Arc<AtomicBool>,
    _task: JoinHandle<()>,
}

pub struct GrabManager {
    inner: tokio::sync::Mutex<Option<GrabHandle>>,
}

impl Default for GrabManager {
    fn default() -> Self {
        Self::new()
    }
}

impl GrabManager {
    pub fn new() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(None),
        }
    }

    pub async fn start(
        &self,
        state: Arc<AppState>,
        targets: Vec<GrabTarget>,
        cfg: GrabConfig,
    ) -> Result<GrabStatus, AppError> {
        if targets.is_empty() {
            return Err(AppError::bad_request("至少选择一个监听目标"));
        }

        let mut guard = self.inner.lock().await;
        if let Some(handle) = guard.as_ref()
            && lock_status(&handle.status).running
        {
            return Err(AppError::bad_request("已有监听任务在运行，请先停止"));
        }

        let status = Arc::new(Mutex::new(GrabStatus::starting(&targets, &cfg)));
        let stop = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(run(state, targets, cfg, status.clone(), stop.clone()));

        *guard = Some(GrabHandle {
            status: status.clone(),
            stop,
            _task: task,
        });

        Ok(lock_status(&status).clone())
    }

    pub async fn status(&self) -> GrabStatus {
        let guard = self.inner.lock().await;
        match guard.as_ref() {
            Some(handle) => lock_status(&handle.status).clone(),
            None => GrabStatus::idle(),
        }
    }

    pub async fn stop(&self) -> GrabStatus {
        let guard = self.inner.lock().await;
        let Some(handle) = guard.as_ref() else {
            return GrabStatus::idle();
        };
        handle.stop.store(true, Ordering::SeqCst);
        let mut status = lock_status(&handle.status);
        if status.running {
            status.running = false;
            status.finished_at_ms = Some(now_ms());
            status.stopped_reason = Some("stopped_by_user".to_string());
            status.last_message = "已手动停止".to_string();
            push_log(&mut status, "stop", "已手动停止".to_string(), None);
        }
        status.clone()
    }
}

fn lock_status(status: &Arc<Mutex<GrabStatus>>) -> MutexGuard<'_, GrabStatus> {
    status.lock().unwrap_or_else(|e| e.into_inner())
}

fn push_log(status: &mut GrabStatus, event: &str, message: String, code: Option<i64>) {
    status.logs.push(GrabLog {
        at_ms: now_ms(),
        event: event.to_string(),
        message,
        code,
    });
    if status.logs.len() > 100 {
        let drop_count = status.logs.len() - 100;
        status.logs.drain(0..drop_count);
    }
}

fn finish(status: &Arc<Mutex<GrabStatus>>, success: bool, reason: &str, message: impl Into<String>) {
    let message = message.into();
    let mut st = lock_status(status);
    st.running = false;
    st.success = success;
    st.finished_at_ms = Some(now_ms());
    st.stopped_reason = Some(reason.to_string());
    st.last_message = message.clone();
    push_log(&mut st, if success { "success" } else { "stop" }, message, None);
}

fn row_available(row: &RawClassRoom) -> bool {
    class_is_available(
        row.capacity.unwrap_or(0),
        row.selected.unwrap_or(0),
        row.round_code.as_deref().unwrap_or(""),
    )
}

async fn run(
    state: Arc<AppState>,
    targets: Vec<GrabTarget>,
    cfg: GrabConfig,
    status: Arc<Mutex<GrabStatus>>,
    stop: Arc<AtomicBool>,
) {
    tracing::info!("监听任务启动: {} 个目标", targets.len());

    // 只有“强制课堂、不检查余量”的单目标模式才跳过课程列表请求。
    let forced_class_only = !cfg.check_capacity_first
        && targets
            .iter()
            .all(|t| matches!(t, GrabTarget::Class(_)));

    loop {
        if stop.load(Ordering::SeqCst) {
            finish(&status, false, "stopped_by_user", "已停止");
            break;
        }

        let reached_limit = {
            let st = lock_status(&status);
            cfg.max_attempts > 0 && st.attempts >= cfg.max_attempts
        };
        if reached_limit {
            finish(&status, false, "max_attempts", "已达到最大尝试次数");
            break;
        }

        {
            let mut st = lock_status(&status);
            st.attempts += 1;
            let attempt = st.attempts;
            // 每 10 次记录一条，避免日志被轮询记录淹没。
            if attempt == 1 || attempt.is_multiple_of(10) {
                push_log(&mut st, "attempt", format!("第 {attempt} 次轮询"), None);
            }
        }

        let session = match state.session(state.refresh_after_ms).await {
            Ok(session) => session,
            Err(err) => {
                if err.code == 401 {
                    finish(&status, false, "session_expired", "登录已失效，请重新登录");
                    break;
                }
                {
                    let mut st = lock_status(&status);
                    push_log(&mut st, "error", err.message.clone(), None);
                    st.last_message = err.message;
                }
                let _ = sleep_interruptible(&stop, cfg.interval_ms).await;
                continue;
            }
        };

        let client = state.client(&session);
        let round_code = session.round.round_code.clone();

        let mut candidate: Option<GrabSelection> = None;

        if forced_class_only {
            if let Some(GrabTarget::Class(code)) = targets.first() {
                candidate = Some(GrabSelection {
                    class_code: code.clone(),
                    course_code: String::new(),
                    course_name: String::new(),
                });
            }
        } else {
            match client.fetch_courses_in_round(&round_code).await {
                Ok(rows) => {
                    'targets: for target in &targets {
                        match target {
                            GrabTarget::Class(code) => {
                                if let Some(row) = rows
                                    .iter()
                                    .find(|r| &r.class_code == code && row_available(r))
                                {
                                    candidate = Some(GrabSelection {
                                        class_code: row.class_code.clone(),
                                        course_code: row.course_code.clone(),
                                        course_name: row.name.clone(),
                                    });
                                    break 'targets;
                                }
                            }
                            GrabTarget::Course(code) => {
                                if let Some(row) = rows
                                    .iter()
                                    .filter(|r| &r.course_code == code && row_available(r))
                                    .max_by_key(|r| {
                                        r.capacity.unwrap_or(0) - r.selected.unwrap_or(0)
                                    })
                                {
                                    candidate = Some(GrabSelection {
                                        class_code: row.class_code.clone(),
                                        course_code: row.course_code.clone(),
                                        course_name: row.name.clone(),
                                    });
                                    break 'targets;
                                }
                            }
                        }
                    }

                    if candidate.is_none() {
                        let wait_msg = format!(
                            "监听中：{} 个目标均暂无余量，继续轮询",
                            targets.len()
                        );
                        let mut st = lock_status(&status);
                        if st.last_message != wait_msg {
                            push_log(&mut st, "wait", wait_msg.clone(), None);
                            st.last_message = wait_msg;
                        }
                    }
                }
                Err(err) => {
                    {
                        let mut st = lock_status(&status);
                        let msg = format!("刷新课程列表失败: {err}");
                        push_log(&mut st, "error", msg.clone(), None);
                        st.last_message = msg;
                    }
                    if sleep_interruptible(&stop, cfg.interval_ms).await {
                        continue;
                    }
                    continue;
                }
            }
        }

        let Some(selection) = candidate else {
            if sleep_interruptible(&stop, cfg.interval_ms).await {
                continue;
            }
            continue;
        };

        {
            let mut st = lock_status(&status);
            st.select_calls += 1;
            push_log(
                &mut st,
                "found",
                format!(
                    "发现可抢：{}（课堂 {}），立即提交",
                    if selection.course_name.is_empty() {
                        &selection.class_code
                    } else {
                        &selection.course_name
                    },
                    selection.class_code
                ),
                None,
            );
        }

        let result = client
            .select_course(&selection.class_code, &round_code)
            .await;

        match result {
            Ok(res) if res.code == 0 => {
                let msg = res
                    .msg
                    .unwrap_or_else(|| format!("选课成功：{}", selection.class_code));
                {
                    let mut st = lock_status(&status);
                    st.last_code = Some(0);
                    st.last_message = msg.clone();
                    st.selection = Some(selection.clone());
                    push_log(&mut st, "success", msg.clone(), Some(0));
                }
                finish(
                    &status,
                    true,
                    "success",
                    format!("{msg}（监听任务已自动结束）"),
                );
                break;
            }
            Ok(res) => {
                let msg = res
                    .msg
                    .unwrap_or_else(|| format!("选课失败（code={}）", res.code));
                {
                    let mut st = lock_status(&status);
                    st.last_code = Some(res.code);
                    st.last_message = msg.clone();
                    push_log(
                        &mut st,
                        "select",
                        format!("{}: {msg}", selection.class_code),
                        Some(res.code),
                    );
                }

                if is_success_message(&msg) {
                    {
                        let mut st = lock_status(&status);
                        st.selection = Some(selection.clone());
                    }
                    finish(
                        &status,
                        true,
                        "already_selected",
                        format!("{msg}（该课程已在课表中，监听任务已自动结束）"),
                    );
                    break;
                }
                if is_fatal_message(&msg) {
                    finish(&status, false, "rejected", msg);
                    break;
                }
            }
            Err(ClientError::NotAuthenticated) => {
                state.sessions.clear().await;
                finish(&status, false, "session_expired", "登录已失效，请重新登录");
                break;
            }
            Err(err) => {
                let mut st = lock_status(&status);
                let msg = format!("请求失败: {err}");
                push_log(&mut st, "error", msg.clone(), None);
                st.last_message = msg;
            }
        }

        if sleep_interruptible(&stop, cfg.interval_ms).await {
            finish(&status, false, "stopped_by_user", "已停止");
            break;
        }
    }

    tracing::info!("监听任务结束");
}

fn is_success_message(msg: &str) -> bool {
    msg.contains("已经选") || msg.contains("已选上") || msg.contains("重复选课")
}

fn is_fatal_message(msg: &str) -> bool {
    [
        "不在本轮",
        "不在选课时间",
        "选课时间未",
        "选课已结束",
        "未开放",
        "没有选课",
        "无选课",
        "登录",
        "认证",
    ]
    .iter()
    .any(|key| msg.contains(key))
}

async fn sleep_interruptible(stop: &Arc<AtomicBool>, interval_ms: u64) -> bool {
    let mut remaining = Duration::from_millis(interval_ms);
    while !remaining.is_zero() {
        if stop.load(Ordering::SeqCst) {
            return true;
        }
        let step = remaining.min(Duration::from_millis(100));
        tokio::time::sleep(step).await;
        remaining = remaining.saturating_sub(step);
    }
    stop.load(Ordering::SeqCst)
}
