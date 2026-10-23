use std::collections::HashMap;

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

// ---------- 上游原始结构 ----------

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Tag {
    #[serde(rename(deserialize = "DM", serialize = "code"), default)]
    pub code: String,
    #[serde(rename(deserialize = "MC", serialize = "name"), default)]
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Schedule {
    #[serde(rename(deserialize = "XM", serialize = "teacher"), default)]
    pub teacher: Option<String>,
    #[serde(rename(deserialize = "JSMC", serialize = "location"), default)]
    pub location: Option<String>,
    #[serde(rename(deserialize = "XQ", serialize = "weekday"), default)]
    pub weekday: Option<String>,
    #[serde(rename(deserialize = "QSJC", serialize = "start_period"), default)]
    pub start_period: Option<String>,
    #[serde(rename(deserialize = "JSJC", serialize = "end_period"), default)]
    pub end_period: Option<String>,
    #[serde(
        rename(deserialize = "QSZC", serialize = "start_week"),
        default,
        deserialize_with = "opt_i64"
    )]
    pub start_week: Option<i64>,
    #[serde(
        rename(deserialize = "JSZC", serialize = "end_week"),
        default,
        deserialize_with = "opt_i64"
    )]
    pub end_week: Option<i64>,
    #[serde(rename(deserialize = "SKXS", serialize = "mode"), default)]
    pub mode: Option<String>,
    #[serde(rename(deserialize = "SKJHBH", serialize = "plan_code"), default)]
    pub plan_code: Option<String>,
}

/// 一行代表一个课堂（教学班）。
#[derive(Clone, Debug, Deserialize)]
pub struct RawClassRoom {
    #[serde(rename = "KTBH")]
    pub class_code: String,
    #[serde(rename = "KTMC", default)]
    pub class_name: Option<String>,
    #[serde(rename = "KCBH", default)]
    pub course_code: String,
    #[serde(rename = "KCDM", default)]
    pub course_id: Option<String>,
    #[serde(rename = "KCMC", default)]
    pub name: String,
    #[serde(rename = "ZXF", default, deserialize_with = "opt_f64")]
    pub credits: Option<f64>,
    #[serde(rename = "KCZXS", default, deserialize_with = "opt_i64")]
    pub hours: Option<i64>,
    #[serde(rename = "DWMC", default)]
    pub department: Option<String>,
    #[serde(rename = "GGKDLBH", default)]
    pub category_code: Option<String>,
    #[serde(rename = "GGKDLMC", default)]
    pub category_name: Option<String>,
    #[serde(rename = "GGKLBBH", default)]
    pub sub_category_code: Option<String>,
    #[serde(rename = "GGKLBMC", default)]
    pub sub_category_name: Option<String>,
    #[serde(rename = "bsList", default)]
    pub tags: Vec<Tag>,
    #[serde(rename = "XKDX", default)]
    pub target: Option<String>,
    #[serde(rename = "xkjddm", default)]
    pub round_code: Option<String>,
    #[serde(rename = "XQH", default)]
    pub term: Option<String>,
    #[serde(rename = "KTRL", default, deserialize_with = "opt_i64")]
    pub capacity: Option<i64>,
    #[serde(rename = "KTRS", default, deserialize_with = "opt_i64")]
    pub selected: Option<i64>,
    #[serde(rename = "skjhlist", default)]
    pub schedules: Vec<Schedule>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct RawSelectedCourse {
    #[serde(rename = "KCMC", default)]
    pub name: String,
    #[serde(rename = "KTBH", default)]
    pub class_code: String,
    #[serde(rename = "KCBH", default)]
    pub course_code: Option<String>,
    #[serde(rename = "KCZXS", default, deserialize_with = "opt_i64")]
    pub hours: Option<i64>,
    #[serde(rename = "KCZXF", default, deserialize_with = "opt_f64")]
    pub credits: Option<f64>,
    #[serde(rename = "XQMC", default)]
    pub term_name: Option<String>,
    #[serde(rename = "XKDX", default)]
    pub target: Option<String>,
    #[serde(rename = "XKJDDM", default)]
    pub round_code: Option<String>,
    #[serde(rename = "SFKTX", default)]
    pub can_drop_flag: Option<String>,
    #[serde(rename = "tips", default)]
    pub drop_tips: Option<String>,
    #[serde(rename = "bsList", default)]
    pub tags: Vec<Tag>,
    #[serde(rename = "skjhlist", default)]
    pub schedules: Vec<Schedule>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpstreamEnvelope<T> {
    pub code: i64,
    #[serde(default)]
    pub msg: Option<String>,
    #[serde(default)]
    pub data: Option<T>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct UpstreamResult {
    pub code: i64,
    #[serde(default)]
    pub msg: Option<String>,
}

// ---------- 对外视图 ----------

#[derive(Clone, Debug, Serialize)]
pub struct ClassView {
    pub class_code: String,
    pub class_name: String,
    pub target: String,
    pub round_code: String,
    pub capacity: i64,
    pub selected: i64,
    pub remaining: i64,
    pub available: bool,
    pub schedules: Vec<Schedule>,
}

#[derive(Clone, Debug, Serialize)]
pub struct CourseView {
    pub course_code: String,
    pub course_id: Option<String>,
    pub name: String,
    pub category_code: Option<String>,
    pub category_name: Option<String>,
    pub sub_category_code: Option<String>,
    pub sub_category_name: Option<String>,
    pub tags: Vec<Tag>,
    pub credits: f64,
    pub hours: i64,
    pub department: String,
    pub term: Option<String>,
    pub capacity: i64,
    pub selected: i64,
    pub remaining: i64,
    pub available: bool,
    pub class_count: usize,
    pub classes: Vec<ClassView>,
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectedCourseView {
    pub course_code: String,
    pub class_code: String,
    pub name: String,
    pub credits: f64,
    pub hours: i64,
    pub term_name: String,
    pub target: String,
    pub round_phase: String,
    pub can_drop: bool,
    pub drop_tips: Option<String>,
    pub tags: Vec<Tag>,
    pub schedules: Vec<Schedule>,
}

/// 课堂是否可选：未满且不属于“0 开头”的非正选轮次。
pub fn class_is_available(capacity: i64, selected: i64, round_code: &str) -> bool {
    capacity > selected && !round_code.starts_with('0')
}

pub fn class_view(raw: &RawClassRoom) -> ClassView {
    let capacity = raw.capacity.unwrap_or(0);
    let selected = raw.selected.unwrap_or(0);
    let round_code = raw.round_code.clone().unwrap_or_default();
    ClassView {
        class_code: raw.class_code.clone(),
        class_name: raw.class_name.clone().unwrap_or_default(),
        target: raw.target.clone().unwrap_or_default(),
        round_code: round_code.clone(),
        capacity,
        selected,
        remaining: (capacity - selected).max(0),
        available: class_is_available(capacity, selected, &round_code),
        schedules: raw.schedules.clone(),
    }
}

pub fn aggregate_courses(rows: Vec<RawClassRoom>) -> Vec<CourseView> {
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut courses: Vec<CourseView> = Vec::new();

    for raw in rows {
        let key = if raw.course_code.is_empty() {
            raw.class_code.clone()
        } else {
            raw.course_code.clone()
        };

        let class = class_view(&raw);
        if let Some(&pos) = index.get(&key) {
            let course = &mut courses[pos];
            course.capacity += class.capacity;
            course.selected += class.selected;
            course.remaining = (course.capacity - course.selected).max(0);
            course.available = course.available || class.available;
            course.class_count += 1;
            course.classes.push(class);
            continue;
        }

        index.insert(key.clone(), courses.len());
        let mut course = CourseView {
            course_code: raw.course_code.clone(),
            course_id: raw.course_id.clone(),
            name: raw.name.clone(),
            category_code: raw.category_code.clone(),
            category_name: raw.category_name.clone(),
            sub_category_code: raw.sub_category_code.clone(),
            sub_category_name: raw.sub_category_name.clone(),
            tags: raw.tags.clone(),
            credits: raw.credits.unwrap_or(0.0),
            hours: raw.hours.unwrap_or(0),
            department: raw.department.clone().unwrap_or_default(),
            term: raw.term.clone(),
            capacity: class.capacity,
            selected: class.selected,
            remaining: class.remaining,
            available: class.available,
            class_count: 1,
            classes: vec![class],
        };
        course.classes.sort_by(|a, b| {
            b.available
                .cmp(&a.available)
                .then(b.remaining.cmp(&a.remaining))
                .then(a.class_code.cmp(&b.class_code))
        });
        courses.push(course);
    }

    for course in &mut courses {
        course.classes.sort_by(|a, b| {
            b.available
                .cmp(&a.available)
                .then(b.remaining.cmp(&a.remaining))
                .then(a.class_code.cmp(&b.class_code))
        });
    }

    courses.sort_by(|a, b| {
        b.available
            .cmp(&a.available)
            .then(b.remaining.cmp(&a.remaining))
            .then(a.name.cmp(&b.name))
    });

    courses
}

pub fn round_phase_name(code: &str) -> String {
    match code {
        "a" => "正选".to_string(),
        "6" => "改选".to_string(),
        "8" => "专场".to_string(),
        "" => "未知".to_string(),
        other => other.to_string(),
    }
}

// ---------- 兼容数字/字符串的字段解析 ----------

fn opt_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f.round() as i64)),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok().or_else(|| {
            s.trim()
                .parse::<f64>()
                .ok()
                .map(|f| f.round() as i64)
        }),
        _ => None,
    })
}

fn opt_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    Ok(match value {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => s.trim().parse::<f64>().ok(),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(class: &str, capacity: i64, selected: i64) -> RawClassRoom {
        RawClassRoom {
            class_code: class.to_string(),
            class_name: Some("课堂".into()),
            course_code: "w1".into(),
            course_id: Some("GEC1".into()),
            name: "测试课程".into(),
            credits: Some(2.0),
            hours: Some(32),
            department: Some("某学院".into()),
            category_code: Some("55".into()),
            category_name: Some("思政".into()),
            sub_category_code: None,
            sub_category_name: None,
            tags: vec![],
            target: Some("一年级本科生".into()),
            round_code: Some("11".into()),
            term: Some("20261".into()),
            capacity: Some(capacity),
            selected: Some(selected),
            schedules: vec![],
        }
    }

    #[test]
    fn aggregates_classes() {
        let courses = aggregate_courses(vec![row("c1", 100, 100), row("c2", 100, 90)]);
        assert_eq!(courses.len(), 1);
        let course = &courses[0];
        assert_eq!(course.capacity, 200);
        assert_eq!(course.selected, 190);
        assert_eq!(course.remaining, 10);
        assert!(course.available);
        assert_eq!(course.class_count, 2);
        assert_eq!(course.classes[0].class_code, "c2");
    }

    #[test]
    fn zero_round_is_not_available() {
        let mut r = row("c1", 100, 0);
        r.round_code = Some("01".into());
        let view = class_view(&r);
        assert!(!view.available);
    }
}
