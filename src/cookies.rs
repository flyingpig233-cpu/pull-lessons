use std::collections::HashSet;

use serde::{Deserialize, Deserializer, Serialize};

pub const DEFAULT_DOMAIN: &str = ".hust.edu.cn";
pub const DEFAULT_PATH: &str = "/";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    #[serde(
        default = "default_domain",
        deserialize_with = "domain_or_default"
    )]
    pub domain: String,
    #[serde(default = "default_path", deserialize_with = "path_or_default")]
    pub path: String,
    #[serde(default)]
    pub secure: bool,
}

fn default_domain() -> String {
    DEFAULT_DOMAIN.to_string()
}

fn default_path() -> String {
    DEFAULT_PATH.to_string()
}

/// Tauri 等框架返回的 cookie 中 domain/path 可能是 null，这里做兜底。
fn domain_or_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_domain))
}

fn path_or_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<String>::deserialize(deserializer)?;
    Ok(value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(default_path))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CookieFormat {
    Auto,
    Netscape,
    Header,
    Json,
}

impl CookieFormat {
    pub fn parse(raw: Option<&str>) -> Result<Self, String> {
        match raw.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            None | Some("") => Ok(Self::Auto),
            Some("auto") => Ok(Self::Auto),
            Some("netscape") | Some("cookiejar") | Some("txt") => Ok(Self::Netscape),
            Some("header") | Some("cookie") | Some("string") => Ok(Self::Header),
            Some("json") => Ok(Self::Json),
            Some(other) => Err(format!("不支持的 cookies 格式: {other}")),
        }
    }
}

/// 支持三种输入：
/// - Netscape cookies.txt（浏览器 / curl 导出的文件内容）
/// - Cookie 请求头字符串（`a=b; c=d`）
/// - JSON 数组（Tauri 等框架读取到的 cookie 列表）
pub fn parse_cookies(input: &str, format: CookieFormat) -> Result<Vec<Cookie>, String> {
    let format = match format {
        CookieFormat::Auto => detect_format(input),
        other => other,
    };

    let cookies = match format {
        CookieFormat::Netscape => parse_netscape(input),
        CookieFormat::Header => parse_header(input),
        CookieFormat::Json => parse_json(input),
        CookieFormat::Auto => unreachable!(),
    }?;

    let cookies = dedupe(cookies);
    if cookies.is_empty() {
        return Err("没有解析出任何 cookie".to_string());
    }
    Ok(cookies)
}

fn detect_format(input: &str) -> CookieFormat {
    let trimmed = input.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        CookieFormat::Json
    } else if input.lines().any(|line| line.contains('\t')) {
        CookieFormat::Netscape
    } else {
        CookieFormat::Header
    }
}

fn parse_netscape(input: &str) -> Result<Vec<Cookie>, String> {
    let mut out = Vec::new();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    for raw_line in input.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.trim().is_empty() {
            continue;
        }

        let line = if let Some(rest) = line.strip_prefix("#HttpOnly_") {
            rest
        } else if line.starts_with('#') {
            continue;
        } else {
            line
        };

        let fields: Vec<&str> = line.split('\t').collect();
        if fields.len() < 7 {
            continue;
        }

        let domain = fields[0].trim();
        let path = fields[2].trim();
        let secure = fields[3].trim().eq_ignore_ascii_case("true");
        let expires: u64 = fields[4].trim().parse().unwrap_or(0);
        if expires != 0 && expires < now {
            continue;
        }

        let name = fields[5].trim();
        let value = fields[6..].join("\t");
        if name.is_empty() {
            continue;
        }

        out.push(Cookie {
            name: name.to_string(),
            value,
            domain: if domain.is_empty() {
                DEFAULT_DOMAIN.to_string()
            } else {
                domain.to_string()
            },
            path: if path.is_empty() {
                DEFAULT_PATH.to_string()
            } else {
                path.to_string()
            },
            secure,
        });
    }

    Ok(out)
}

fn parse_header(input: &str) -> Result<Vec<Cookie>, String> {
    let mut out = Vec::new();
    for part in input.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((name, value)) = part.split_once('=') else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || name.starts_with('$') {
            continue;
        }
        out.push(Cookie {
            name: name.to_string(),
            value: value.trim().to_string(),
            domain: DEFAULT_DOMAIN.to_string(),
            path: DEFAULT_PATH.to_string(),
            secure: false,
        });
    }
    Ok(out)
}

fn parse_json(input: &str) -> Result<Vec<Cookie>, String> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Wrapper {
        List(Vec<Cookie>),
        Object { cookies: Vec<Cookie> },
    }

    let wrapper: Wrapper =
        serde_json::from_str(input.trim()).map_err(|e| format!("cookie JSON 解析失败: {e}"))?;
    Ok(match wrapper {
        Wrapper::List(list) => list,
        Wrapper::Object { cookies } => cookies,
    })
}

fn dedupe(cookies: Vec<Cookie>) -> Vec<Cookie> {
    let mut seen = HashSet::new();
    let mut out: Vec<Cookie> = Vec::new();
    for cookie in cookies.into_iter().rev() {
        let key = (
            cookie.name.clone(),
            normalize_domain(&cookie.domain),
            cookie.path.clone(),
        );
        if seen.insert(key) {
            out.push(cookie);
        }
    }
    out.reverse();
    out
}

fn normalize_domain(domain: &str) -> String {
    domain.trim().trim_start_matches('.').to_ascii_lowercase()
}

fn domain_matches(cookie_domain: &str, host: &str) -> bool {
    let domain = normalize_domain(cookie_domain);
    if domain.is_empty() {
        return false;
    }
    let host = host.to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn path_matches(cookie_path: &str, request_path: &str) -> bool {
    let cookie_path = if cookie_path.is_empty() {
        "/"
    } else {
        cookie_path
    };
    if cookie_path == "/" {
        return true;
    }
    request_path == cookie_path || request_path.starts_with(&format!("{cookie_path}/"))
}

/// 按域名和路径筛选 cookie，拼成 Cookie 请求头。
pub fn cookie_header(cookies: &[Cookie], host: &str, request_path: &str) -> String {
    let mut pairs: Vec<String> = Vec::new();
    for cookie in cookies {
        if !domain_matches(&cookie.domain, host) {
            continue;
        }
        if !path_matches(&cookie.path, request_path) {
            continue;
        }
        pairs.push(format!("{}={}", cookie.name, cookie.value));
    }
    pairs.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# Netscape HTTP Cookie File\n\
.hust.edu.cn\tTRUE\t/\tFALSE\t0\tUID\t185195912\n\
#HttpOnly_bksxk.hust.edu.cn\tFALSE\t/\tFALSE\t0\tJSESSIONID\tabc!123\n";

    #[test]
    fn parses_netscape_and_http_only() {
        let cookies = parse_cookies(SAMPLE, CookieFormat::Auto).unwrap();
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].name, "UID");
        assert_eq!(cookies[1].name, "JSESSIONID");
        assert_eq!(cookies[1].domain, "bksxk.hust.edu.cn");
    }

    #[test]
    fn parses_header_and_defaults_domain() {
        let cookies = parse_cookies("JSESSIONID=abc; BIGip=1.2.3", CookieFormat::Auto).unwrap();
        assert_eq!(cookies.len(), 2);
        assert_eq!(cookies[0].domain, DEFAULT_DOMAIN);
    }

    #[test]
    fn builds_matching_header() {
        let cookies = parse_cookies(SAMPLE, CookieFormat::Auto).unwrap();
        let header = cookie_header(&cookies, "bksxk.hust.edu.cn", "/zxqStuCSS/findZxqCourses");
        assert!(header.contains("UID=185195912"));
        assert!(header.contains("JSESSIONID=abc!123"));
    }

    #[test]
    fn dedupes_by_name_domain_path() {
        let input = "a=1; a=2";
        let cookies = parse_cookies(input, CookieFormat::Header).unwrap();
        assert_eq!(cookies.len(), 1);
        assert_eq!(cookies[0].value, "2");
    }

    #[test]
    fn parses_json() {
        let input = r#"[{"name":"JSESSIONID","value":"x","domain":"bksxk.hust.edu.cn","path":"/"}]"#;
        let cookies = parse_cookies(input, CookieFormat::Auto).unwrap();
        assert_eq!(cookies[0].name, "JSESSIONID");
    }

    #[test]
    fn parses_json_with_null_domain_and_extra_fields() {
        let input = r#"[{"name":"JSESSIONID","value":"x","domain":null,"path":null,
            "secure":false,"httpOnly":true,"sameSite":null,"expires":null}]"#;
        let cookies = parse_cookies(input, CookieFormat::Json).unwrap();
        assert_eq!(cookies[0].domain, DEFAULT_DOMAIN);
        assert_eq!(cookies[0].path, DEFAULT_PATH);
    }
}
