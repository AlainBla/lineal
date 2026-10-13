//! Prozess-Test (B3.10): `serve` und `worker` beenden sich auf SIGTERM geordnet mit Exit 0
//! (statt nach `docker stop` mit SIGKILL/137).

#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use sqlx::PgPool;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

fn database_url(pool: &PgPool) -> String {
    let db = pool
        .connect_options()
        .get_database()
        .expect("test database name")
        .to_owned();
    let base = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://lineal:lineal@localhost:5440/lineal".into());
    let cut = base.rfind('/').expect("database url");
    format!("{}/{db}", &base[..cut])
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn spawn(cmd: &str, url: &str, bind: Option<u16>) -> Child {
    let mut c = Command::new(env!("CARGO_BIN_EXE_lineal"));
    c.arg(cmd)
        .env("DATABASE_URL", url)
        .env("LINEAL_DEV", "1")
        .env("RUST_LOG", "info")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(port) = bind {
        c.env("LINEAL_BIND", format!("127.0.0.1:{port}"));
    }
    c.spawn().expect("spawn lineal")
}

fn wait_exit(child: &mut Child, within: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    while start.elapsed() < within {
        if let Some(status) = child.try_wait().unwrap() {
            return Some(status);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    None
}

fn sigterm(child: &Child) {
    let status = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

fn output(child: &mut Child) -> String {
    let mut out = String::new();
    if let Some(mut s) = child.stdout.take() {
        let _ = s.read_to_string(&mut out);
    }
    if let Some(mut s) = child.stderr.take() {
        let _ = s.read_to_string(&mut out);
    }
    out
}

fn healthz(port: u16) -> bool {
    let Ok(mut s) = TcpStream::connect(("127.0.0.1", port)) else {
        return false;
    };
    let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = s.write_all(b"GET /healthz HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    let mut buf = String::new();
    let _ = s.read_to_string(&mut buf);
    buf.starts_with("HTTP/1.1 200")
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn serve_exits_cleanly_on_sigterm(pool: PgPool) {
    let url = database_url(&pool);
    let port = free_port();
    let mut child = spawn("serve", &url, Some(port));
    let start = Instant::now();
    while !healthz(port) {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "server did not start"
        );
        assert!(child.try_wait().unwrap().is_none(), "server exited early");
        std::thread::sleep(Duration::from_millis(100));
    }
    sigterm(&child);
    let status = wait_exit(&mut child, Duration::from_secs(15));
    if status.is_none() {
        let _ = child.kill();
    }
    let out = output(&mut child);
    let status = status.expect("serve must exit on SIGTERM");
    assert!(status.success(), "exit {status:?}\n{out}");
    assert!(out.contains("shutting down"), "{out}");
}

#[sqlx::test(migrations = "../lineal-db/migrations")]
async fn worker_exits_cleanly_on_sigterm(pool: PgPool) {
    let url = database_url(&pool);
    let mut child = spawn("worker", &url, None);
    // Warten, bis der Worker läuft (Signal-Handler sind nach dem Start installiert).
    std::thread::sleep(Duration::from_secs(3));
    assert!(child.try_wait().unwrap().is_none(), "worker exited early");
    sigterm(&child);
    let status = wait_exit(&mut child, Duration::from_secs(15));
    if status.is_none() {
        let _ = child.kill();
    }
    let out = output(&mut child);
    let status = status.expect("worker must exit on SIGTERM");
    assert!(status.success(), "exit {status:?}\n{out}");
    assert!(out.contains("shutting down"), "{out}");
}
