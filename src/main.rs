use std::net::SocketAddr;
use std::path::PathBuf;

use pull_lessons::{init_tracing, serve};

struct Args {
    addr: SocketAddr,
    cookies: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    init_tracing();

    let args = parse_args()?;
    serve(args.addr, args.cookies).await?;
    Ok(())
}

fn parse_args() -> Result<Args, Box<dyn std::error::Error + Send + Sync>> {
    let mut addr = std::env::var("PULL_LESSONS_ADDR")
        .ok()
        .and_then(|value| value.parse::<SocketAddr>().ok())
        .unwrap_or_else(|| SocketAddr::from(([127, 0, 0, 1], 8787)));

    let mut cookies = std::env::var("PULL_LESSONS_COOKIES")
        .ok()
        .map(PathBuf::from);

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--addr" => {
                let value = args.next().ok_or("--addr 缺少参数")?;
                addr = value.parse::<SocketAddr>()?;
            }
            "--cookies" => {
                let value = args.next().ok_or("--cookies 缺少参数")?;
                cookies = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                println!(
                    "pull-lessons backend\n\n\
                     用法:\n  pull-lessons [--addr 127.0.0.1:8787] [--cookies <cookies.txt>]\n\n\
                     环境变量:\n  PULL_LESSONS_ADDR     监听地址，默认 127.0.0.1:8787\n\
                     PULL_LESSONS_COOKIES  cookies 文件路径\n\
                     RUST_LOG              日志级别，默认 pull_lessons=info,tower_http=warn"
                );
                std::process::exit(0);
            }
            other => return Err(format!("未知参数: {other}（使用 --help 查看用法）").into()),
        }
    }

    // 开发便利：未显式指定时，如果 D:\cookies.txt 存在就自动加载。
    if cookies.is_none() {
        let dev_default = PathBuf::from(r"D:\cookies.txt");
        if dev_default.is_file() {
            tracing::info!("未指定 cookies，自动使用开发文件 D:\\cookies.txt");
            cookies = Some(dev_default);
        }
    }

    Ok(Args { addr, cookies })
}
