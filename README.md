# pull-lessons

华中科技大学主校区公选课（网上选课系统 `bksxk.hust.edu.cn`）的抢课工具，包含 Tauri 桌面 GUI 和 Rust 后端。

后端不处理密码登录：桌面端弹出学校统一身份认证页面，用户登录成功后由前端读取 webview 的 cookie，回传给后端。后端用这份 cookie 维持会话，提供课程查询、选课、退选和自动抢课接口。

## 桌面应用（推荐）

```bash
cargo run -p pull-lessons-desktop
```

桌面应用会在进程内启动后端（`127.0.0.1:8787`），首次启动显示登录页：

1. 点击“打开学校登录窗口”，在弹窗中完成统一身份认证；
2. 登录成功后自动读取 webview 的 cookie 并同步给后端，进入主界面；
3. 主界面列出所有可选课程及人数/容量，勾选多门课程加入监听；
4. 启动监听后，后端每隔一段时间轮询课程余量，任意一门课程出现余量就立即提交选课；
5. 抢到后弹出“抢课成功”提示，任务自动结束。

界面还支持课程搜索、按公共课大类筛选、只看有余量、查看课堂明细、立即选课、查看已选课程和退选。

会话会保存在本机，下次启动应用自动恢复，无需重复登录。

## 打包发布

```bash
npm install
npx tauri build
```

产物：

- `target/release/pull-lessons-desktop.exe`
- `target/release/bundle/nsis/pull-lessons_0.1.0_x64-setup.exe`

安装包按当前用户安装，无需管理员权限。默认使用 `downloadBootstrapper`，系统缺少 WebView2 时会尝试安装。

已发布的 Windows x64 安装包在 [GitHub Releases](https://github.com/flyingpig233-cpu/pull-lessons/releases/tag/v0.1.0)：

- `pull-lessons_0.1.0_x64-setup.exe`：Windows 安装包
- `pull-lessons_0.1.0_windows_x64_portable.zip`：便携版（解压运行 `pull-lessons-desktop.exe`）
- `SHA256SUMS.txt`：校验和

发布前打包好的文件放在本地 `dist/`（已加入 `.gitignore`，不会提交），也可以直接从 `target/release/` 取用。

安装包未做代码签名，Windows SmartScreen 可能提示“未知发布者”，需要选择“更多信息 → 仍要运行”。如果要公开发布，建议使用代码签名证书对 exe 和安装包签名。

如果所在网络无法访问 GitHub，Tauri 打包会在下载 NSIS 工具链时卡住。可以手动把 `nsis-3.13.zip` 解压到 `%LOCALAPPDATA%\tauri\NSIS`，并把 `nsis_tauri_utils.dll` 放到 `Plugins/x86-unicode/additional/`；目录中需要包含 `NSIS.exe`、`Bin/makensis.exe` 等文件。

## 后端单独运行

如果只想跑后端 API：

```bash
# 开发模式：加载你提供的 cookies.txt
cargo run -- --cookies D:/cookies.txt

# 或者用环境变量
PULL_LESSONS_COOKIES=D:/cookies.txt cargo run

# 生产构建
cargo build --release
```

默认监听 `127.0.0.1:8787`。开发时如果没有显式指定 `--cookies`，程序会自动尝试加载 `D:\cookies.txt`。

健康检查：

```bash
curl http://127.0.0.1:8787/api/health
```

其他参数：

| 参数 | 说明 |
| --- | --- |
| `--addr 127.0.0.1:8787` | 监听地址 |
| `--cookies <path>` | 启动时加载 cookies 文件并校验登录 |
| `PULL_LESSONS_ADDR` | 监听地址环境变量 |
| `PULL_LESSONS_COOKIES` | cookies 文件环境变量 |
| `RUST_LOG` | 日志级别，默认 `pull_lessons=info,tower_http=warn` |

登录成功后会将会话持久化到 `%APPDATA%\pull-lessons\session.json`，重启后端会自动恢复并校验。

## 登录流程

桌面端已经实现完整流程：

1. 点击“打开学校登录窗口”，Tauri 创建独立窗口打开 `https://bksxk.hust.edu.cn/login`。
2. 用户在窗口内完成统一身份认证，学校页面跳转回 `bksxk.hust.edu.cn`。
3. Rust 端的 `on_page_load` 检测到登录后的页面，通知主窗口。
4. 主窗口调用 `login_from_webview`，用 `WebviewWindow::cookies()` 读取 webview 的全部 cookie（包括 HttpOnly 和 `.hust.edu.cn` 共享 cookie），POST 到 `/api/auth/cookies`。
5. 后端访问选课页校验会话，成功后持久化并返回用户信息，主界面自动加载课程。

自己实现前端时，也可以直接调用 `GET /api/auth/login-url` 获取登录地址，然后 POST 三种格式的 cookie：

```json
{ "cookies": "# Netscape HTTP Cookie File\n...", "format": "netscape" }
```

```json
{ "cookies": "JSESSIONID=...; BIGipServerpool-hub-bksxk=...", "format": "header" }
```

```json
{
  "cookies": [
    { "name": "JSESSIONID", "value": "...", "domain": "bksxk.hust.edu.cn", "path": "/" }
  ],
  "format": "json"
}
```

> 实测注意：只传 `bksxk.hust.edu.cn` 的 `JSESSIONID` 不够，学校接口会返回
> “您不在本轮改选对应的正选可选对象中”。必须同时带上 `.hust.edu.cn` 的共享 cookie
> （`1731userinfo`、`UID`、`p_auth_token` 等）。因此要读取 webview 的全部 cookie，
> 而不是只取当前 URL 的 cookie。

任意业务接口返回 HTTP 401 时，表示会话失效，前端应重新走登录流程。

后端只允许以下来源跨域访问（防止任意网页调用本地端口触发选课）：

- `tauri://localhost`
- `http://tauri.localhost` / `https://tauri.localhost`
- `http://localhost:1420` / `http://127.0.0.1:1420`（Tauri 默认开发端口）

如果前端开发端口不是 1420，需要在 `src/lib.rs` 的 `cors_layer()` 里追加。

## 统一响应格式

```json
{ "code": 0, "msg": "ok", "data": { } }
```

| code | HTTP | 含义 |
| --- | --- | --- |
| `0` | 200 | 成功 |
| `-1` | 200 | 业务失败（学校系统拒绝，`msg` 为学校返回的原因） |
| `-2` | 400 | 请求参数错误 |
| `401` | 401 | 未登录或登录失效 |
| `502` | 502 | 访问学校服务器失败 |

## API

### 登录与会话

| 方法 | 路径 | 说明 |
| --- | --- | --- |
| GET | `/api/auth/login-url` | 返回登录页地址和需要收集 cookie 的域名 |
| POST | `/api/auth/cookies` | 回传 cookie，校验成功后建立会话 |
| GET | `/api/auth/status` | 查看当前是否已登录、用户、学期、轮次 |
| POST | `/api/auth/validate` | 强制回源校验并刷新轮次信息 |
| POST | `/api/auth/logout` | 清除会话并停止抢课任务 |

`POST /api/auth/cookies` 请求体：

```json
{
  "cookies": [{ "name": "JSESSIONID", "value": "...", "domain": "bksxk.hust.edu.cn", "path": "/" }],
  "format": "json"
}
```

`format` 支持 `auto`（默认）、`netscape`、`header`、`json`。

成功响应：

```json
{
  "code": 0,
  "data": {
    "authenticated": true,
    "user": { "name": "徐承敏", "account": "U202615212" },
    "round": { "term": "20261", "round_code": "11", "batch_id": "XKLC0000000201" },
    "select_path": "/zxqStuCSS/readyToCourseSS?cdbh=13860",
    "selected_path": "/zxqStuCSS/readyToQuerySelectedCourses?cdbh=11485",
    "validated_at_ms": 1792626905131,
    "cookie_count": 18
  }
}
```

`round_code`、`batch_id` 和两个页面路径由后端从学校页面动态解析，学期或选课阶段切换后会自动刷新（默认 30 秒）。

### 元信息

`GET /api/meta`

```json
{
  "code": 0,
  "data": {
    "term": "20261",
    "round_code": "11",
    "batch_id": "XKLC0000000201",
    "categories": [{ "code": "55", "name": "思政选择性必修课" }],
    "weeks": [1, 2, 3, "...", 19],
    "select_path": "/zxqStuCSS/readyToCourseSS?cdbh=13860",
    "selected_path": "/zxqStuCSS/readyToQuerySelectedCourses?cdbh=11485"
  }
}
```

### 课程列表

`GET /api/courses`

查询参数（全部可选）：

| 参数 | 说明 |
| --- | --- |
| `keyword` | 模糊匹配课程名、课程代码、院系、教师、标签、课堂号 |
| `category` | 公共课大类编码或名称，例如 `55` 或 `思政` |
| `tag` | 公共课标识，例如 `05`（思政类） |
| `weekday` | 上课星期，例如 `周六` |
| `department` | 开课院系模糊匹配 |
| `only_available` | `1` / `true` 只看有余量的课程 |
| `sort` | `available`（默认，有余量优先）、`remaining`、`name` |

响应中每个课程聚合了它下属的所有课堂：

```json
{
  "code": 0,
  "data": {
    "term": "20261",
    "round_code": "11",
    "total": 28,
    "available_count": 0,
    "total_remaining": 0,
    "courses": [
      {
        "course_code": "w908729",
        "course_id": "GEC0207",
        "name": "“新医科”中华传统文化经典选读（思政）",
        "category_code": "55",
        "category_name": "思政选择性必修课",
        "sub_category_code": null,
        "sub_category_name": null,
        "tags": [{ "code": "05", "name": "思政类" }],
        "credits": 2.0,
        "hours": 32,
        "department": "公共卫生学院",
        "term": "20261",
        "capacity": 200,
        "selected": 200,
        "remaining": 0,
        "available": false,
        "class_count": 2,
        "classes": [
          {
            "class_code": "20261w908729001",
            "class_name": "一年级本科生",
            "target": "一年级本科生",
            "round_code": "11",
            "capacity": 100,
            "selected": 100,
            "remaining": 0,
            "available": false,
            "schedules": [
              {
                "teacher": "梁渊",
                "location": "西五楼119",
                "weekday": "周六",
                "start_period": "1",
                "end_period": "4",
                "start_week": 9,
                "end_week": 16,
                "mode": "线下教学",
                "plan_code": "20261w908729001E199700576"
              }
            ]
          }
        ]
      }
    ]
  }
}
```

字段说明：

- `capacity` / `selected` / `remaining`：该课程所有课堂的容量、已选人数、余量。
- `available`：是否存在“未满且属于当前可选轮次”的课堂（与学校页面“选修”按钮的可用逻辑一致）。
- `classes[].class_code`：选课和抢课要用的课堂编号（`KTBH`）。
- `schedules`：教师、地点、星期、节次、周次。

`GET /api/courses/{course_code}/classes` 单独查询某门课的最新课堂和人数。

### 已选课程

`GET /api/selected`

```json
{
  "code": 0,
  "data": {
    "term": "20261",
    "batch_id": "XKLC0000000201",
    "total": 0,
    "courses": []
  }
}
```

每条记录包含 `course_code`、`class_code`、`name`、`credits`、`hours`、`term_name`、`target`、`round_phase`、`can_drop`、`drop_tips`、`tags`、`schedules`。

### 选课 / 退选

`POST /api/select`

```json
{ "class_code": "20261w908729001" }
```

`POST /api/drop`

```json
{ "class_code": "20261w908729001" }
```

成功：

```json
{
  "code": 0,
  "msg": "选课成功",
  "data": {
    "success": true,
    "class_code": "20261w908729001",
    "upstream_code": 0,
    "upstream_msg": "选课成功"
  }
}
```

失败时 `code` 为 `-1`，`msg` 为学校返回的原因，例如：

```json
{
  "code": -1,
  "msg": "选课失败！此课堂不在您的可选范围内，请刷新后重试",
  "data": { "success": false, "upstream_code": -1 }
}
```

### 自动抢课

`POST /api/grab/start`

一次可以监听多门课程，数组顺序就是抢课优先级：

```json
{
  "targets": [
    { "kind": "course", "value": "w908729" },
    { "kind": "course", "value": "w908731" }
  ],
  "interval_ms": 800,
  "max_attempts": 0,
  "check_capacity_first": true
}
```

也兼容单目标写法：

```json
{
  "class_code": "20261w908729001",
  "interval_ms": 800,
  "check_capacity_first": true
}
```

| 字段 | 说明 |
| --- | --- |
| `targets` | 监听目标数组，`kind` 为 `course`（课程）或 `class`（课堂），`value` 为课程号或课堂号 |
| `class_code` | 兼容字段：指定课堂，追加到目标列表 |
| `course_code` | 兼容字段：指定课程，追加到目标列表 |
| `interval_ms` | 轮询间隔，默认 800，最小 300，最大 60000 |
| `max_attempts` | 最大轮询次数，`0` 表示不限制 |
| `check_capacity_first` | 默认 `true`：先查课程列表确认有余量才提交选课，避免无意义的选课请求 |

对于课程目标，后端会自动选择该课程下余量最大的可选课堂。

`GET /api/grab/status`

```json
{
  "code": 0,
  "data": {
    "running": true,
    "targets": [
      { "kind": "course", "value": "w908729" },
      { "kind": "course", "value": "w908731" }
    ],
    "attempts": 12,
    "select_calls": 0,
    "started_at_ms": 1792626967245,
    "finished_at_ms": null,
    "success": false,
    "selection": null,
    "last_code": null,
    "last_message": "监听中：2 个目标均暂无余量，继续轮询",
    "stopped_reason": null,
    "logs": [
      { "at_ms": 1792626967245, "event": "start", "message": "开始监听 2 个目标（课程 w908729、课程 w908731），间隔 800ms，最多 不限 次", "code": null }
    ]
  }
}
```

抢课成功后：

```json
{
  "running": false,
  "success": true,
  "selection": {
    "class_code": "20261w908729001",
    "course_code": "w908729",
    "course_name": "“新医科”中华传统文化经典选读（思政）"
  },
  "stopped_reason": "success"
}
```

任务结束原因（`stopped_reason`）：

| 值 | 含义 |
| --- | --- |
| `success` | 选课成功 |
| `already_selected` | 学校返回“已选/重复选课”，视为达成目标 |
| `max_attempts` | 达到最大轮询次数 |
| `stopped_by_user` | 用户手动停止 |
| `session_expired` | 登录失效 |
| `rejected` | 学校返回不可重试的原因（不在选课时间、轮次不符等） |

`POST /api/grab/stop` 停止当前任务。

同一时间只允许一个监听任务，但一个任务可以包含多个目标。监听任务遇到第一个成功选课的目标（学校返回成功，或返回“已选/重复选课”）就会立即停止，返回 `success: true` 和 `selection`，不会继续提交其他课程。任务会每 30 秒刷新一次会话和选课轮次，跨越选课阶段也能继续工作。

## 学校接口对应关系

后端对 `https://bksxk.hust.edu.cn` 的调用：

| 用途 | 接口 |
| --- | --- |
| 登录态 / 用户 / 菜单 | `GET /index?xkxt=zxq&cdbh=11326` |
| 选课页（解析 `xqh`、`xkjddm`） | `GET /zxqStuCSS/readyToCourseSS?cdbh=13860` |
| 已选页（解析 `xklcid`） | `GET /zxqStuCSS/readyToQuerySelectedCourses?cdbh=11485` |
| 课程 + 课堂 + 人数容量 | `POST /zxqStuCSS/findClassRoomsForSelect` |
| 某课程的课堂 | `POST /zxqStuCSS/findZxqClassRooms` |
| 已选课程 | `POST /zxqStuCSS/querySelectedCourses` |
| 选课 | `POST /zxqStuCSS/selectCourse`（form: `ktbh`、`xkjddm`） |
| 退选 | `POST /zxqStuCSS/dropCourse`（form: `ktbh`、`xqh`、`xklcid`） |
| 公共课大类 | `POST /CommonController/getGxkList` |
| 周次 | `POST /zxqStuCSS/findWeekListByXqh` |

两个课程查询接口必须带 `page` 和 `limit`，否则学校服务器会误报“您不在本轮改选对应的正选可选对象中”。

## 代码结构

```
Cargo.toml          工作区 + 后端 crate
src/
├── lib.rs        后端库入口（serve / 日志 / CORS）
├── main.rs       后端命令行入口
├── api.rs        HTTP 路由与请求/响应模型
├── client.rs     学校系统 HTTP 客户端、页面解析
├── models.rs     上游字段映射、课程聚合、对外视图
├── cookies.rs    cookies.txt / Cookie 头 / JSON 解析与域匹配
├── session.rs    会话存储与本地持久化
├── state.rs      共享状态、会话刷新
├── grab.rs       多目标监听任务管理
└── error.rs      统一错误与响应

desktop/          Tauri 桌面应用
├── src/main.rs   登录窗口、cookie 读取与同步
├── tauri.conf.json
└── capabilities/
ui/
├── index.html    登录页 + 主界面
├── styles.css
└── app.js        课程列表、监听队列、状态轮询、成功提示
```

## 测试

```bash
cargo test
cargo clippy --all-targets
node --check ui/app.js
```

## 使用提醒

- 抢课接口会以你的账号真实提交选课，请使用合理的轮询间隔（默认 800ms），不要高频轰炸学校服务器。
- 学校接口和页面结构可能随学期变化；如果解析失败，日志会指出具体是 `xqh`、`xkjddm` 还是 `xklcid`。
- 本工具只用于个人选课，请遵守学校相关规定。
