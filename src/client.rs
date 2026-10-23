use std::sync::LazyLock;

use regex::Regex;
use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, REFERER};
use serde::de::DeserializeOwned;
use serde_json::json;

use crate::cookies::{Cookie, cookie_header};
use crate::error::ClientError;
use crate::models::{RawClassRoom, RawSelectedCourse, Tag, UpstreamEnvelope, UpstreamResult};

pub const HOST: &str = "bksxk.hust.edu.cn";
pub const BASE_URL: &str = "https://bksxk.hust.edu.cn";

pub const DEFAULT_SELECT_PATH: &str = "/zxqStuCSS/readyToCourseSS?cdbh=13860";
pub const DEFAULT_SELECTED_PATH: &str = "/zxqStuCSS/readyToQuerySelectedCourses?cdbh=11485";

const INDEX_PATH: &str = "/index?xkxt=zxq&cdbh=11326";
const PATH_COURSES: &str = "/zxqStuCSS/findClassRoomsForSelect";
const PATH_COURSE_CLASSES: &str = "/zxqStuCSS/findZxqClassRooms";
const PATH_SELECTED: &str = "/zxqStuCSS/querySelectedCourses";
const PATH_SELECT: &str = "/zxqStuCSS/selectCourse";
const PATH_DROP: &str = "/zxqStuCSS/dropCourse";
const PATH_CATEGORIES: &str = "/CommonController/getGxkList";
const PATH_WEEKS: &str = "/zxqStuCSS/findWeekListByXqh";

static RE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"姓名[：:]\s*([^<\s]+)").expect("valid regex"));
static RE_ACCOUNT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"账号[：:]\s*([^<\s]+)").expect("valid regex"));
static RE_TERM: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:var|let|const)\s+xqh\s*=\s*['"]([^'"]*)['"]"#).expect("valid regex")
});
static RE_ROUND: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:var|let|const)\s+xkjddm\s*=\s*['"]([^'"]*)['"]"#).expect("valid regex")
});
static RE_BATCH: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:var|let|const)\s+xklcid\s*=\s*['"]([^'"]*)['"]"#).expect("valid regex")
});
static RE_HREF: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-href="([^"]+)""#).expect("valid regex"));

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct User {
    pub name: String,
    pub account: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Round {
    pub term: String,
    pub round_code: String,
    pub batch_id: String,
}

#[derive(Clone, Debug)]
pub struct Validated {
    pub user: User,
    pub round: Round,
    pub select_path: String,
    pub selected_path: String,
}

pub struct HustClient {
    http: reqwest::Client,
    cookies: Vec<Cookie>,
    base: String,
    select_path: String,
    selected_path: String,
}

impl HustClient {
    pub fn new(http: reqwest::Client, cookies: Vec<Cookie>) -> Self {
        Self {
            http,
            cookies,
            base: BASE_URL.to_string(),
            select_path: DEFAULT_SELECT_PATH.to_string(),
            selected_path: DEFAULT_SELECTED_PATH.to_string(),
        }
    }

    pub fn with_paths(
        http: reqwest::Client,
        cookies: Vec<Cookie>,
        select_path: impl Into<String>,
        selected_path: impl Into<String>,
    ) -> Self {
        Self {
            http,
            cookies,
            base: BASE_URL.to_string(),
            select_path: select_path.into(),
            selected_path: selected_path.into(),
        }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else if path.starts_with('/') {
            format!("{}{}", self.base, path)
        } else {
            format!("{}/{}", self.base, path)
        }
    }

    fn request_path(path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            reqwest::Url::parse(path)
                .map(|u| u.path().to_string())
                .unwrap_or_else(|_| "/".to_string())
        } else {
            path.split('?').next().unwrap_or("/").to_string()
        }
    }

    fn cookie_header_for(&self, path: &str) -> String {
        cookie_header(&self.cookies, HOST, &Self::request_path(path))
    }

    fn select_referer(&self) -> String {
        self.url(&self.select_path)
    }

    async fn get_html(&self, path: &str) -> Result<String, ClientError> {
        let resp = self
            .http
            .get(self.url(path))
            .header(COOKIE, self.cookie_header_for(path))
            .header(
                ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header(REFERER, self.url("/"))
            .send()
            .await?;

        if resp.status().is_redirection() {
            return Err(ClientError::NotAuthenticated);
        }
        if !resp.status().is_success() {
            return Err(ClientError::Upstream(format!(
                "学校服务器返回 HTTP {} ({path})",
                resp.status()
            )));
        }
        Ok(resp.text().await?)
    }

    async fn post_json<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<T, ClientError> {
        let resp = self
            .http
            .post(self.url(path))
            .header(COOKIE, self.cookie_header_for(path))
            .header(CONTENT_TYPE, "application/json")
            .header(ACCEPT, "application/json, text/javascript, */*; q=0.01")
            .header("X-Requested-With", "XMLHttpRequest")
            .header(REFERER, self.select_referer())
            .json(body)
            .send()
            .await?;
        self.handle_response(path, resp).await
    }

    async fn post_form<T: DeserializeOwned>(
        &self,
        path: &str,
        params: &[(&str, &str)],
        referer: &str,
    ) -> Result<T, ClientError> {
        let resp = self
            .http
            .post(self.url(path))
            .header(COOKIE, self.cookie_header_for(path))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(ACCEPT, "application/json, text/javascript, */*; q=0.01")
            .header("X-Requested-With", "XMLHttpRequest")
            .header(REFERER, referer)
            .form(params)
            .send()
            .await?;
        self.handle_response(path, resp).await
    }

    async fn handle_response<T: DeserializeOwned>(
        &self,
        path: &str,
        resp: reqwest::Response,
    ) -> Result<T, ClientError> {
        if resp.status().is_redirection() {
            return Err(ClientError::NotAuthenticated);
        }
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            || resp.status() == reqwest::StatusCode::FORBIDDEN
        {
            return Err(ClientError::NotAuthenticated);
        }
        if !resp.status().is_success() {
            return Err(ClientError::Upstream(format!(
                "学校服务器返回 HTTP {} ({path})",
                resp.status()
            )));
        }

        let text = resp.text().await?;
        let trimmed = text.trim_start();
        if trimmed.starts_with('<') {
            if text.contains("统一身份认证") || text.contains("登录") {
                return Err(ClientError::NotAuthenticated);
            }
            return Err(ClientError::Upstream(format!(
                "学校服务器返回了 HTML 而不是 JSON ({path})"
            )));
        }

        serde_json::from_str::<T>(&text).map_err(|e| {
            let snippet: String = text.chars().take(200).collect();
            ClientError::Upstream(format!("解析学校服务器响应失败 ({path}): {e}; 内容: {snippet}"))
        })
    }

    /// 用给定 cookie 校验登录态，并解析学期、选课轮次、选课批次等信息。
    pub async fn validate(&self) -> Result<Validated, ClientError> {
        let index = self.get_html(INDEX_PATH).await?;
        let Some(user) = parse_user(&index) else {
            return Err(ClientError::NotAuthenticated);
        };

        let (select_path, selected_path) = parse_menu(&index);

        let select_page = self.get_html(&select_path).await?;
        let term = parse_term(&select_page).ok_or_else(|| {
            ClientError::Parse("无法从选课页面解析学期 xqh".to_string())
        })?;
        let round_code = parse_round(&select_page).ok_or_else(|| {
            ClientError::Parse("无法从选课页面解析选课轮次 xkjddm".to_string())
        })?;

        let selected_page = self.get_html(&selected_path).await?;
        let batch_id = parse_batch(&selected_page).ok_or_else(|| {
            ClientError::Parse("无法从已选课程页面解析选课批次 xklcid".to_string())
        })?;

        Ok(Validated {
            user,
            round: Round {
                term,
                round_code,
                batch_id,
            },
            select_path,
            selected_path,
        })
    }

    pub async fn fetch_courses_in_round(
        &self,
        round_code: &str,
    ) -> Result<Vec<RawClassRoom>, ClientError> {
        // 学校接口必须带 page/limit，否则会误报“不在本轮可选对象中”。
        let body = json!({ "xkjddm": round_code, "page": 1, "limit": 200 });
        let envelope: UpstreamEnvelope<Vec<RawClassRoom>> = self.post_json(PATH_COURSES, &body).await?;
        if envelope.code != 0 {
            return Err(ClientError::Upstream(
                envelope
                    .msg
                    .unwrap_or_else(|| format!("查询课程失败（code={}）", envelope.code)),
            ));
        }
        Ok(envelope.data.unwrap_or_default())
    }

    pub async fn fetch_course_classes(
        &self,
        course_code: &str,
        round_code: &str,
    ) -> Result<Vec<RawClassRoom>, ClientError> {
        let body = json!({
            "xkjddm": round_code,
            "kcbh": course_code,
            "page": 1,
            "limit": 200,
        });
        let envelope: UpstreamEnvelope<Vec<RawClassRoom>> =
            self.post_json(PATH_COURSE_CLASSES, &body).await?;
        if envelope.code != 0 {
            return Err(ClientError::Upstream(
                envelope
                    .msg
                    .unwrap_or_else(|| format!("查询课堂失败（code={}）", envelope.code)),
            ));
        }
        Ok(envelope.data.unwrap_or_default())
    }

    pub async fn fetch_selected(
        &self,
        term: &str,
        batch_id: &str,
    ) -> Result<Vec<RawSelectedCourse>, ClientError> {
        let body = json!({
            "xklcid": batch_id,
            "xqh": term,
            "page": 1,
            "limit": 200,
        });
        let envelope: UpstreamEnvelope<Vec<RawSelectedCourse>> =
            self.post_json(PATH_SELECTED, &body).await?;
        if envelope.code != 0 {
            return Err(ClientError::Upstream(
                envelope
                    .msg
                    .unwrap_or_else(|| format!("查询已选课程失败（code={}）", envelope.code)),
            ));
        }
        Ok(envelope.data.unwrap_or_default())
    }

    pub async fn select_course(
        &self,
        class_code: &str,
        round_code: &str,
    ) -> Result<UpstreamResult, ClientError> {
        let referer = self.select_referer();
        self.post_form(
            PATH_SELECT,
            &[("ktbh", class_code), ("xkjddm", round_code)],
            &referer,
        )
        .await
    }

    pub async fn drop_course(
        &self,
        class_code: &str,
        term: &str,
        batch_id: &str,
    ) -> Result<UpstreamResult, ClientError> {
        let referer = self.url(&self.selected_path);
        self.post_form(
            PATH_DROP,
            &[
                ("ktbh", class_code),
                ("xqh", term),
                ("xklcid", batch_id),
            ],
            &referer,
        )
        .await
    }

    pub async fn fetch_categories(&self) -> Result<Vec<Tag>, ClientError> {
        let envelope: UpstreamEnvelope<Vec<Tag>> =
            self.post_json(PATH_CATEGORIES, &json!({})).await?;
        if envelope.code != 0 {
            return Err(ClientError::Upstream(
                envelope.msg.unwrap_or_else(|| "查询公共课大类失败".to_string()),
            ));
        }
        Ok(envelope.data.unwrap_or_default())
    }

    pub async fn fetch_weeks(&self, term: &str) -> Result<Vec<i64>, ClientError> {
        let referer = self.select_referer();
        let envelope: UpstreamEnvelope<Vec<i64>> = self
            .post_form(PATH_WEEKS, &[("xqh", term)], &referer)
            .await?;
        if envelope.code != 0 {
            return Err(ClientError::Upstream(
                envelope.msg.unwrap_or_else(|| "查询周次失败".to_string()),
            ));
        }
        Ok(envelope.data.unwrap_or_default())
    }
}

pub fn parse_user(html: &str) -> Option<User> {
    let name = RE_NAME.captures(html)?.get(1)?.as_str().trim().to_string();
    let account = RE_ACCOUNT
        .captures(html)?
        .get(1)?
        .as_str()
        .trim()
        .to_string();
    if name.is_empty() || account.is_empty() {
        return None;
    }
    Some(User { name, account })
}

pub fn parse_menu(html: &str) -> (String, String) {
    let mut select_path = DEFAULT_SELECT_PATH.to_string();
    let mut selected_path = DEFAULT_SELECTED_PATH.to_string();

    for cap in RE_HREF.captures_iter(html) {
        let Some(href) = cap.get(1) else { continue };
        let href = href.as_str();
        if href.contains("readyToCourseSS") {
            select_path = href.to_string();
        } else if href.contains("readyToQuerySelectedCourses") {
            selected_path = href.to_string();
        }
    }
    (select_path, selected_path)
}

pub fn parse_term(html: &str) -> Option<String> {
    RE_TERM
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .filter(|s| !s.is_empty())
}

pub fn parse_round(html: &str) -> Option<String> {
    RE_ROUND
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .filter(|s| !s.is_empty())
}

pub fn parse_batch(html: &str) -> Option<String> {
    RE_BATCH
        .captures(html)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_user_from_index() {
        let html = r#"<span>姓名：徐承敏</span><span>账号：U202615212</span>"#;
        let user = parse_user(html).unwrap();
        assert_eq!(user.name, "徐承敏");
        assert_eq!(user.account, "U202615212");
    }

    #[test]
    fn parses_round_and_batch() {
        let html = r#"var xqh = '20261'; var xkjddm = '11'; var xklcid = 'XKLC0000000201';"#;
        assert_eq!(parse_term(html).as_deref(), Some("20261"));
        assert_eq!(parse_round(html).as_deref(), Some("11"));
        assert_eq!(parse_batch(html).as_deref(), Some("XKLC0000000201"));
    }

    #[test]
    fn parses_menu_links() {
        let html = r#"
            <a href="javascript:;" data-href="/zxqStuCSS/readyToCourseSS?cdbh=13860">选课</a>
            <a href="javascript:;" data-href="/zxqStuCSS/readyToQuerySelectedCourses?cdbh=11485">已选</a>
        "#;
        let (select, selected) = parse_menu(html);
        assert_eq!(select, "/zxqStuCSS/readyToCourseSS?cdbh=13860");
        assert_eq!(selected, "/zxqStuCSS/readyToQuerySelectedCourses?cdbh=11485");
    }
}
