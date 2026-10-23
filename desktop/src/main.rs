#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const BACKEND: &str = "http://127.0.0.1:8787";
const LOGIN_URL: &str = "https://bksxk.hust.edu.cn/login";

struct LoginState {
    busy: AtomicBool,
}

fn is_logged_in_url(url: &tauri::Url) -> bool {
    if url.scheme() != "https" {
        return false;
    }
    let host = url.host_str().unwrap_or_default();
    if host != "bksxk.hust.edu.cn" {
        return false;
    }
    let path = url.path();
    // 登录页本身和 CAS 认证页不算登录完成。
    !path.starts_with("/login") && !path.starts_with("/cas")
}

#[tauri::command]
async fn start_login(app: AppHandle) -> Result<(), String> {
    if let Some(state) = app.try_state::<LoginState>() {
        state.busy.store(false, Ordering::SeqCst);
    }

    if let Some(window) = app.get_webview_window("login") {
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let url = LOGIN_URL.parse::<tauri::Url>().map_err(|e| e.to_string())?;
    let app_handle = app.clone();

    WebviewWindowBuilder::new(&app, "login", WebviewUrl::External(url))
        .title("登录 · 华中科技大学统一身份认证")
        .inner_size(1000.0, 780.0)
        .min_inner_size(860.0, 640.0)
        .center()
        .on_page_load(move |_window, payload| {
            let url = payload.url().clone();
            if !is_logged_in_url(&url) {
                return;
            }
            let state = app_handle.state::<LoginState>();
            // 登录成功后的页面可能触发多次加载，只通知一次。
            if state.busy.swap(true, Ordering::SeqCst) {
                return;
            }
            let _ = app_handle.emit_to(
                "main",
                "login-ready",
                json!({ "url": url.to_string() }),
            );
        })
        .build()
        .map_err(|e| format!("创建登录窗口失败: {e}"))?;

    Ok(())
}

#[tauri::command]
async fn login_from_webview(app: AppHandle) -> Result<Value, String> {    let mut last_error = String::new();

    for attempt in 0..3 {
        let app_for_cookies = app.clone();
        // Windows 上同步读取 cookie 会死锁，必须放到阻塞线程。
        let cookies = tauri::async_runtime::spawn_blocking(move || {
            let window = app_for_cookies
                .get_webview_window("login")
                .or_else(|| app_for_cookies.get_webview_window("main"))
                .ok_or_else(|| "登录窗口已关闭".to_string())?;
            window.cookies().map_err(|e| e.to_string())
        })
        .await
        .map_err(|e| e.to_string())??;

        if cookies.is_empty() && attempt < 2 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            continue;
        }

        let list: Vec<Value> = cookies
            .iter()
            .map(|cookie| {
                json!({
                    "name": cookie.name(),
                    "value": cookie.value(),
                    "domain": cookie.domain(),
                    "path": cookie.path(),
                    "secure": cookie.secure(),
                })
            })
            .collect();

        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        let response = client
            .post(format!("{BACKEND}/api/auth/cookies"))
            .json(&json!({ "cookies": list, "format": "json" }))
            .send()
            .await
            .map_err(|e| format!("无法连接本地后端: {e}"))?;

        let status = response.status();
        let text = response.text().await.map_err(|e| e.to_string())?;
        let parsed: Value = serde_json::from_str(&text)
            .map_err(|_| format!("后端响应异常: {}", text.chars().take(160).collect::<String>()))?;

        if parsed.get("code").and_then(Value::as_i64) == Some(0) {
            let data = parsed.get("data").cloned().unwrap_or(Value::Null);
            if let Some(state) = app.try_state::<LoginState>() {
                state.busy.store(false, Ordering::SeqCst);
            }
            if let Some(window) = app.get_webview_window("login") {
                let _ = window.close();
            }
            let _ = app.emit_to("main", "login-success", data.clone());
            return Ok(data);
        }

        last_error = parsed
            .get("msg")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("登录失败（HTTP {status}）"));

        if attempt < 2 {
            tokio::time::sleep(Duration::from_millis(600)).await;
        }
    }

    if let Some(state) = app.try_state::<LoginState>() {
        state.busy.store(false, Ordering::SeqCst);
    }
    let _ = app.emit_to("main", "login-failed", json!({ "message": last_error }));
    Err(last_error)
}

fn main() {
    pull_lessons::init_tracing();

    tauri::Builder::default()
        .manage(LoginState {
            busy: AtomicBool::new(false),
        })
        .invoke_handler(tauri::generate_handler![start_login, login_from_webview])
        .setup(|_app| {
            // 后端与 GUI 同进程启动，省去单独运行一个可执行文件。
            tauri::async_runtime::spawn(async move {
                match pull_lessons::serve("127.0.0.1:8787".parse().unwrap(), None).await {
                    Ok(()) => tracing::info!("内置后端已停止"),
                    Err(err) => {
                        tracing::warn!("内置后端启动失败（可能已有实例在运行）: {err}")
                    }
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Tauri 应用启动失败");
}
