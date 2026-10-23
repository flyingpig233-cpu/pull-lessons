use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::client::{HustClient, Validated};
use crate::error::{AppError, ClientError, now_ms};
use crate::grab::GrabManager;
use crate::session::{Session, SessionStore};

pub struct AppState {
    pub http: reqwest::Client,
    pub sessions: SessionStore,
    pub grab: GrabManager,
    pub refresh_lock: Mutex<()>,
    /// 会话缓存的刷新阈值，避免每次请求都回源校验。
    pub refresh_after_ms: u64,
}

impl AppState {
    pub fn new() -> Arc<Self> {
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .user_agent(
                "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
            )
            .timeout(Duration::from_secs(20))
            .build()
            .expect("构建 HTTP 客户端失败");

        Arc::new(Self {
            http,
            sessions: SessionStore::new(),
            grab: GrabManager::new(),
            refresh_lock: Mutex::new(()),
            refresh_after_ms: 30_000,
        })
    }

    pub fn client(&self, session: &Session) -> HustClient {
        HustClient::with_paths(
            self.http.clone(),
            session.cookies.clone(),
            session.select_path.clone(),
            session.selected_path.clone(),
        )
    }

    /// 获取当前会话；超过 max_age_ms 未校验时会回源刷新（选课轮次可能在期间切换）。
    pub async fn session(&self, max_age_ms: u64) -> Result<Session, AppError> {
        let Some(current) = self.sessions.get().await else {
            return Err(AppError::unauthorized());
        };

        if now_ms().saturating_sub(current.validated_at_ms) <= max_age_ms {
            return Ok(current);
        }

        let _guard = self.refresh_lock.lock().await;

        // 拿到锁后重新检查，避免并发请求重复回源。
        let Some(current) = self.sessions.get().await else {
            return Err(AppError::unauthorized());
        };
        if now_ms().saturating_sub(current.validated_at_ms) <= max_age_ms {
            return Ok(current);
        }

        let client = self.client(&current);
        match client.validate().await {
            Ok(validated) => {
                let session = session_from(validated, current.cookies.clone());
                self.sessions.set(session.clone()).await;
                tracing::info!(
                    "会话已刷新: {} / 学期 {} / 轮次 {}",
                    session.user.account,
                    session.round.term,
                    session.round.round_code
                );
                Ok(session)
            }
            Err(ClientError::NotAuthenticated) => {
                self.sessions.clear().await;
                Err(AppError::unauthorized())
            }
            Err(err) => {
                tracing::warn!("刷新会话失败，继续使用已缓存会话: {err}");
                Ok(current)
            }
        }
    }

    pub async fn refresh_session(&self, current: &Session) -> Result<Session, AppError> {
        let client = self.client(current);
        match client.validate().await {
            Ok(validated) => {
                let session = session_from(validated, current.cookies.clone());
                self.sessions.set(session.clone()).await;
                Ok(session)
            }
            Err(ClientError::NotAuthenticated) => {
                self.sessions.clear().await;
                Err(AppError::unauthorized())
            }
            Err(err) => Err(err.into()),
        }
    }
}

pub fn session_from(validated: Validated, cookies: Vec<crate::cookies::Cookie>) -> Session {
    Session {
        cookies,
        user: validated.user,
        round: validated.round,
        select_path: validated.select_path,
        selected_path: validated.selected_path,
        validated_at_ms: now_ms(),
    }
}
