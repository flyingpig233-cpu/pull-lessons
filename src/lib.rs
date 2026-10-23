pub mod api;
pub mod client;
pub mod cookies;
pub mod error;
pub mod grab;
pub mod models;
pub mod session;
pub mod state;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::HeaderValue;
use tokio::net::TcpListener;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::client::HustClient;
use crate::cookies::{CookieFormat, parse_cookies};
use crate::state::{AppState, session_from};

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("pull_lessons=info,tower_http=warn")),
        )
        .with_target(false)
        .try_init();
}

/// 启动后端 HTTP 服务。Tauri 应用和独立可执行文件共用这个入口。
pub async fn serve(
    addr: SocketAddr,
    cookies: Option<PathBuf>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = AppState::new();

    if let Some(path) = cookies {
        spawn_cookie_login(state.clone(), path);
    } else if state.sessions.is_authenticated().await {
        let state = state.clone();
        tokio::spawn(async move {
            if let Some(current) = state.sessions.get().await {
                match state.refresh_session(&current).await {
                    Ok(session) => tracing::info!(
                        "已恢复登录会话: {} ({})",
                        session.user.name,
                        session.user.account
                    ),
                    Err(err) => tracing::warn!("恢复登录会话失败: {}", err.message),
                }
            }
        });
    }

    let app = api::router(state)
        .layer(cors_layer())
        .layer(TraceLayer::new_for_http());

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("pull-lessons backend 已启动: http://{addr}");
    tracing::info!("API 前缀: /api，健康检查: /api/health");

    axum::serve(listener, app).await?;
    Ok(())
}

fn cors_layer() -> CorsLayer {
    // 只允许 Tauri 生产/开发环境跨域，避免任意网页调用本地端口触发选课。
    CorsLayer::new()
        .allow_origin(AllowOrigin::list([
            HeaderValue::from_static("tauri://localhost"),
            HeaderValue::from_static("http://tauri.localhost"),
            HeaderValue::from_static("https://tauri.localhost"),
            HeaderValue::from_static("http://localhost:1420"),
            HeaderValue::from_static("http://127.0.0.1:1420"),
        ]))
        .allow_methods(Any)
        .allow_headers(Any)
}

fn spawn_cookie_login(state: Arc<AppState>, path: PathBuf) {
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) => {
            tracing::warn!("读取 cookie 文件 {} 失败: {err}", path.display());
            return;
        }
    };

    tokio::spawn(async move {
        let cookies = match parse_cookies(&text, CookieFormat::Auto) {
            Ok(cookies) => cookies,
            Err(err) => {
                tracing::warn!("解析 cookie 文件失败: {err}");
                return;
            }
        };
        let client = HustClient::new(state.http.clone(), cookies.clone());
        match client.validate().await {
            Ok(validated) => {
                let session = session_from(validated, cookies);
                tracing::info!(
                    "cookie 文件登录成功: {} ({})",
                    session.user.name,
                    session.user.account
                );
                state.sessions.set(session).await;
            }
            Err(err) => tracing::warn!("cookie 文件校验失败: {err}"),
        }
    });
}
