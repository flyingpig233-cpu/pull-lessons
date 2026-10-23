use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::client::HustClient;
use crate::cookies::{Cookie, CookieFormat, parse_cookies};
use crate::error::{AppError, ok};
use crate::grab::{GrabConfig, GrabStatus, GrabTarget};
use crate::models::{
    CourseView, RawClassRoom, SelectedCourseView, aggregate_courses, class_view, round_phase_name,
};
use crate::session::Session;
use crate::state::{AppState, session_from};

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(health))
        .route("/api/auth/login-url", get(login_url))
        .route("/api/auth/cookies", post(auth_cookies))
        .route("/api/auth/status", get(auth_status))
        .route("/api/auth/validate", post(auth_validate))
        .route("/api/auth/logout", post(auth_logout))
        .route("/api/meta", get(meta))
        .route("/api/courses", get(list_courses))
        .route("/api/courses/{course_code}/classes", get(course_classes))
        .route("/api/selected", get(list_selected))
        .route("/api/select", post(select_course))
        .route("/api/drop", post(drop_course))
        .route("/api/grab/start", post(grab_start))
        .route("/api/grab/status", get(grab_status))
        .route("/api/grab/stop", post(grab_stop))
        .with_state(state)
}

// ---------- 基础 ----------

async fn health() -> Json<Value> {
    ok(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

async fn login_url() -> Json<Value> {
    ok(json!({
        "url": format!("{}/login", crate::client::BASE_URL),
        "login_path": "/login",
        "host": crate::client::HOST,
        "cookie_domains": [".hust.edu.cn", "bksxk.hust.edu.cn"],
        "note": "在 Tauri 登录窗口完成统一身份认证后，读取 webview 的全部 cookie 并 POST 到 /api/auth/cookies",
    }))
}

// ---------- 登录 ----------

#[derive(Deserialize)]
#[serde(untagged)]
pub enum CookieInput {
    Text(String),
    List(Vec<Cookie>),
    Wrapped { cookies: Vec<Cookie> },
}

impl Default for CookieInput {
    fn default() -> Self {
        Self::Text(String::new())
    }
}

#[derive(Deserialize)]
pub struct CookieLoginRequest {
    #[serde(default)]
    pub cookies: CookieInput,
    #[serde(default)]
    pub format: Option<String>,
}

async fn auth_cookies(
    State(state): State<Arc<AppState>>,
    Json(req): Json<CookieLoginRequest>,
) -> Result<Json<Value>, AppError> {
    let format = CookieFormat::parse(req.format.as_deref()).map_err(AppError::bad_request)?;

    let text = match &req.cookies {
        CookieInput::Text(text) => text.clone(),
        CookieInput::List(list) => serde_json::to_string(list)
            .map_err(|e| AppError::bad_request(format!("cookie 列表序列化失败: {e}")))?,
        CookieInput::Wrapped { cookies } => serde_json::to_string(cookies)
            .map_err(|e| AppError::bad_request(format!("cookie 列表序列化失败: {e}")))?,
    };

    if text.trim().is_empty() {
        return Err(AppError::bad_request(
            "请提供 cookies（Netscape cookies.txt 内容、Cookie 请求头或 cookie JSON 数组）",
        ));
    }

    let cookies = parse_cookies(&text, format).map_err(AppError::bad_request)?;
    tracing::info!("收到 {} 个 cookie，开始校验登录态", cookies.len());

    let client = HustClient::new(state.http.clone(), cookies.clone());
    let validated = client.validate().await?;
    let session = session_from(validated, cookies);

    if state.sessions.is_authenticated().await {
        let _ = state.grab.stop().await;
    }
    state.sessions.set(session.clone()).await;

    tracing::info!(
        "登录成功: {} ({})",
        session.user.name,
        session.user.account
    );
    Ok(ok(session_json(&session)))
}

async fn auth_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    let grab = state.grab.status().await;
    match state.sessions.get().await {
        Some(session) => ok(session_json(&session)),
        None => ok(json!({
            "authenticated": false,
            "user": null,
            "round": null,
            "grab": grab,
        })),
    }
}

async fn auth_validate(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let current = state
        .sessions
        .get()
        .await
        .ok_or_else(AppError::unauthorized)?;
    let session = state.refresh_session(&current).await?;
    Ok(ok(session_json(&session)))
}

async fn auth_logout(State(state): State<Arc<AppState>>) -> Json<Value> {
    let _ = state.grab.stop().await;
    state.sessions.clear().await;
    ok(json!({ "authenticated": false }))
}

fn session_json(session: &Session) -> Value {
    json!({
        "authenticated": true,
        "user": session.user,
        "round": {
            "term": session.round.term,
            "round_code": session.round.round_code,
            "batch_id": session.round.batch_id,
        },
        "select_path": session.select_path,
        "selected_path": session.selected_path,
        "validated_at_ms": session.validated_at_ms,
        "cookie_count": session.cookies.len(),
    })
}

// ---------- 元信息 ----------

async fn meta(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let session = state.session(state.refresh_after_ms).await?;
    let client = state.client(&session);

    let (categories, weeks) = tokio::join!(
        client.fetch_categories(),
        client.fetch_weeks(&session.round.term)
    );

    Ok(ok(json!({
        "term": session.round.term,
        "round_code": session.round.round_code,
        "batch_id": session.round.batch_id,
        "categories": categories.map_err(AppError::from)?,
        "weeks": weeks.map_err(AppError::from)?,
        "select_path": session.select_path,
        "selected_path": session.selected_path,
    })))
}

// ---------- 课程 ----------

#[derive(Debug, Default, Deserialize)]
pub struct CourseQuery {
    pub keyword: Option<String>,
    /// 公共课大类编码，例如 55；也支持名称模糊匹配。
    pub category: Option<String>,
    /// 公共课标识编码，例如 05（思政类）。
    pub tag: Option<String>,
    /// 上课星期，例如 周六。
    pub weekday: Option<String>,
    pub department: Option<String>,
    /// 1/true 表示只看有余量的课程。
    pub only_available: Option<String>,
    /// available（默认）| remaining | name
    pub sort: Option<String>,
}

async fn list_courses(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CourseQuery>,
) -> Result<Json<Value>, AppError> {
    let session = state.session(state.refresh_after_ms).await?;
    let client = state.client(&session);
    let rows = client
        .fetch_courses_in_round(&session.round.round_code)
        .await?;

    let mut courses = aggregate_courses(rows);
    courses.retain(|course| course_matches(course, &query));

    match query.sort.as_deref().unwrap_or("available") {
        "name" => courses.sort_by(|a, b| a.name.cmp(&b.name)),
        "remaining" => courses.sort_by_key(|c| std::cmp::Reverse(c.remaining)),
        _ => {}
    }

    let available_count = courses.iter().filter(|c| c.available).count();
    let total_remaining: i64 = courses.iter().map(|c| c.remaining).sum();

    Ok(ok(json!({
        "term": session.round.term,
        "round_code": session.round.round_code,
        "total": courses.len(),
        "available_count": available_count,
        "total_remaining": total_remaining,
        "courses": courses,
    })))
}

fn course_matches(course: &CourseView, query: &CourseQuery) -> bool {
    if let Some(keyword) = query.keyword.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let keyword = keyword.to_lowercase();
        let mut haystacks: Vec<String> = vec![
            course.name.to_lowercase(),
            course.course_code.to_lowercase(),
            course.department.to_lowercase(),
            course.category_name.clone().unwrap_or_default().to_lowercase(),
            course.sub_category_name.clone().unwrap_or_default().to_lowercase(),
        ];
        if let Some(course_id) = &course.course_id {
            haystacks.push(course_id.to_lowercase());
        }
        for tag in &course.tags {
            haystacks.push(tag.name.to_lowercase());
        }
        for class in &course.classes {
            haystacks.push(class.class_code.to_lowercase());
            for schedule in &class.schedules {
                if let Some(teacher) = &schedule.teacher {
                    haystacks.push(teacher.to_lowercase());
                }
            }
        }
        if !haystacks.iter().any(|item| item.contains(&keyword)) {
            return false;
        }
    }

    if let Some(category) = query.category.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let category = category.to_lowercase();
        let code_match = course
            .category_code
            .as_deref()
            .map(|c| c.eq_ignore_ascii_case(&category))
            .unwrap_or(false);
        let name_match = course
            .category_name
            .as_deref()
            .map(|n| n.to_lowercase().contains(&category))
            .unwrap_or(false);
        if !code_match && !name_match {
            return false;
        }
    }

    if let Some(tag) = query.tag.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let tag = tag.to_lowercase();
        let matched = course.tags.iter().any(|t| {
            t.code.eq_ignore_ascii_case(&tag) || t.name.to_lowercase().contains(&tag)
        });
        if !matched {
            return false;
        }
    }

    if let Some(weekday) = query
        .weekday
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let matched = course.classes.iter().any(|class| {
            class
                .schedules
                .iter()
                .any(|s| s.weekday.as_deref().map(|w| w.contains(weekday)).unwrap_or(false))
        });
        if !matched {
            return false;
        }
    }

    if let Some(department) = query
        .department
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        && !course.department.contains(department)
    {
        return false;
    }

    if query
        .only_available
        .as_deref()
        .map(truthy)
        .unwrap_or(false)
        && !course.available
    {
        return false;
    }

    true
}

fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

async fn course_classes(
    State(state): State<Arc<AppState>>,
    Path(course_code): Path<String>,
) -> Result<Json<Value>, AppError> {
    let session = state.session(state.refresh_after_ms).await?;
    let client = state.client(&session);
    let rows: Vec<RawClassRoom> = client
        .fetch_course_classes(&course_code, &session.round.round_code)
        .await?;
    let classes: Vec<_> = rows.iter().map(class_view).collect();
    let capacity: i64 = classes.iter().map(|c| c.capacity).sum();
    let selected: i64 = classes.iter().map(|c| c.selected).sum();

    Ok(ok(json!({
        "course_code": course_code,
        "capacity": capacity,
        "selected": selected,
        "remaining": (capacity - selected).max(0),
        "classes": classes,
    })))
}

// ---------- 已选 / 退选 ----------

async fn list_selected(State(state): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let session = state.session(state.refresh_after_ms).await?;
    let client = state.client(&session);
    let rows = client
        .fetch_selected(&session.round.term, &session.round.batch_id)
        .await?;

    let courses: Vec<SelectedCourseView> = rows
        .iter()
        .map(|row| SelectedCourseView {
            course_code: row.course_code.clone().unwrap_or_default(),
            class_code: row.class_code.clone(),
            name: row.name.clone(),
            credits: row.credits.unwrap_or(0.0),
            hours: row.hours.unwrap_or(0),
            term_name: row.term_name.clone().unwrap_or_default(),
            target: row.target.clone().unwrap_or_default(),
            round_phase: round_phase_name(row.round_code.as_deref().unwrap_or("")),
            can_drop: row.can_drop_flag.as_deref() == Some("1"),
            drop_tips: row.drop_tips.clone(),
            tags: row.tags.clone(),
            schedules: row.schedules.clone(),
        })
        .collect();

    Ok(ok(json!({
        "term": session.round.term,
        "batch_id": session.round.batch_id,
        "total": courses.len(),
        "courses": courses,
    })))
}

#[derive(Deserialize)]
pub struct ClassRequest {
    #[serde(alias = "ktbh")]
    pub class_code: String,
}

async fn select_course(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClassRequest>,
) -> Result<Json<Value>, AppError> {
    if req.class_code.trim().is_empty() {
        return Err(AppError::bad_request("class_code 不能为空"));
    }
    let session = state.session(state.refresh_after_ms).await?;
    let client = state.client(&session);
    let result = client
        .select_course(req.class_code.trim(), &session.round.round_code)
        .await?;

    let success = result.code == 0;
    let msg = result.msg.unwrap_or_else(|| {
        if success {
            "选课成功".to_string()
        } else {
            "选课失败".to_string()
        }
    });

    Ok(Json(json!({
        "code": if success { 0 } else { -1 },
        "msg": msg,
        "data": {
            "success": success,
            "class_code": req.class_code.trim(),
            "upstream_code": result.code,
            "upstream_msg": msg,
        },
    })))
}

async fn drop_course(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ClassRequest>,
) -> Result<Json<Value>, AppError> {
    if req.class_code.trim().is_empty() {
        return Err(AppError::bad_request("class_code 不能为空"));
    }
    let session = state.session(state.refresh_after_ms).await?;
    if session.round.batch_id.is_empty() {
        return Err(AppError::bad_request("缺少选课批次 xklcid，请重新登录"));
    }
    let client = state.client(&session);
    let result = client
        .drop_course(
            req.class_code.trim(),
            &session.round.term,
            &session.round.batch_id,
        )
        .await?;

    let success = result.code == 0;
    let msg = result.msg.unwrap_or_else(|| {
        if success {
            "退选成功".to_string()
        } else {
            "退选失败".to_string()
        }
    });

    Ok(Json(json!({
        "code": if success { 0 } else { -1 },
        "msg": msg,
        "data": {
            "success": success,
            "class_code": req.class_code.trim(),
            "upstream_code": result.code,
            "upstream_msg": msg,
        },
    })))
}

// ---------- 抢课 ----------

#[derive(Debug, Deserialize)]
pub struct GrabStartRequest {
    /// 监听目标列表，例如 [{"kind":"course","value":"w908729"}]。
    #[serde(default)]
    pub targets: Vec<GrabTarget>,
    /// 兼容单目标调用。
    #[serde(default)]
    pub class_code: Option<String>,
    #[serde(default)]
    pub course_code: Option<String>,
    #[serde(default)]
    pub interval_ms: Option<u64>,
    /// 0 表示不限制。
    #[serde(default)]
    pub max_attempts: Option<u64>,
    /// 默认 true：先查课程列表确认有余量再提交，减少无效请求。
    #[serde(default)]
    pub check_capacity_first: Option<bool>,
}

async fn grab_start(
    State(state): State<Arc<AppState>>,
    Json(req): Json<GrabStartRequest>,
) -> Result<Json<Value>, AppError> {
    let _ = state.session(state.refresh_after_ms).await?;

    let mut targets = req.targets;
    if let Some(code) = req
        .class_code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        targets.push(GrabTarget::Class(code.to_string()));
    }
    if let Some(code) = req
        .course_code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        targets.push(GrabTarget::Course(code.to_string()));
    }

    // 去重并保持用户勾选顺序（顺序即抢课优先级）。
    let mut seen = std::collections::HashSet::new();
    targets.retain(|target| match target {
        GrabTarget::Class(code) => seen.insert((0u8, code.clone())),
        GrabTarget::Course(code) => seen.insert((1u8, code.clone())),
    });

    if targets.is_empty() {
        return Err(AppError::bad_request(
            "请提供 targets，或 class_code / course_code",
        ));
    }

    let interval_ms = req.interval_ms.unwrap_or(800).clamp(300, 60_000);
    let cfg = GrabConfig {
        interval_ms,
        max_attempts: req.max_attempts.unwrap_or(0),
        check_capacity_first: req.check_capacity_first.unwrap_or(true),
    };

    let status: GrabStatus = state.grab.start(state.clone(), targets, cfg).await?;
    Ok(ok(status))
}

async fn grab_status(State(state): State<Arc<AppState>>) -> Json<Value> {
    ok(state.grab.status().await)
}

async fn grab_stop(State(state): State<Arc<AppState>>) -> Json<Value> {
    ok(state.grab.stop().await)
}
