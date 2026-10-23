use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::client::{Round, User};
use crate::cookies::Cookie;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Session {
    pub cookies: Vec<Cookie>,
    pub user: User,
    pub round: Round,
    pub select_path: String,
    pub selected_path: String,
    pub validated_at_ms: u64,
}

/// 单用户桌面应用的会话存储，同时把会话持久化到本地，重启后可直接恢复。
pub struct SessionStore {
    inner: RwLock<Option<Session>>,
    file: Option<PathBuf>,
}

impl Default for SessionStore {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionStore {
    pub fn new() -> Self {
        let file = session_file();
        let session = file
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Session>(&text).ok());

        if session.is_some() {
            tracing::info!("已从本地恢复登录会话");
        }

        Self {
            inner: RwLock::new(session),
            file,
        }
    }

    pub async fn get(&self) -> Option<Session> {
        self.inner.read().await.clone()
    }

    pub async fn is_authenticated(&self) -> bool {
        self.inner.read().await.is_some()
    }

    pub async fn set(&self, session: Session) {
        {
            let mut guard = self.inner.write().await;
            *guard = Some(session.clone());
        }
        self.persist(Some(&session)).await;
    }

    pub async fn clear(&self) {
        {
            let mut guard = self.inner.write().await;
            *guard = None;
        }
        self.persist(None).await;
    }

    async fn persist(&self, session: Option<&Session>) {
        let Some(path) = &self.file else { return };
        if let Some(dir) = path.parent() {
            let _ = tokio::fs::create_dir_all(dir).await;
        }
        match session {
            Some(session) => {
                if let Ok(text) = serde_json::to_string_pretty(session)
                    && let Err(e) = tokio::fs::write(path, text).await
                {
                    tracing::warn!("保存会话失败: {e}");
                }
            }
            None => {
                if let Err(e) = tokio::fs::remove_file(path).await
                    && e.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!("删除会话文件失败: {e}");
                }
            }
        }
    }
}

fn session_file() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("pull-lessons").join("session.json"))
}
