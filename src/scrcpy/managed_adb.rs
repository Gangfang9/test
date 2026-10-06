//! Cancellable ADB CLI operations. Timeout always kills and reaps the client.
use crate::config::LocalConfig;
use std::{
    io::Read,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

const OUTPUT_LIMIT: usize = 64 * 1024;

fn drain(mut reader: impl Read) -> Vec<u8> {
    let mut result = Vec::new();
    let mut buf = [0u8; 4096];
    while let Ok(n) = reader.read(&mut buf) {
        if n == 0 {
            break;
        }
        result.extend_from_slice(&buf[..n]);
        if result.len() > OUTPUT_LIMIT {
            result.drain(..result.len() - OUTPUT_LIMIT);
        }
    }
    result
}

pub async fn run(
    device: Option<&str>,
    args: Vec<String>,
    limit: Option<Duration>,
    token: CancellationToken,
) -> Result<String, String> {
    let path = LocalConfig::get().adb_path;
    let device = device.map(str::to_string);
    tokio::task::spawn_blocking(move || {
        if token.is_cancelled() {
            return Err("ADB 操作已取消".into());
        }
        let mut command = Command::new(path);
        if let Some(device) = device {
            command.args(["-s", &device]);
        }
        command.args(args);
        execute(command, limit, token)
    })
    .await
    .map_err(|e| format!("ADB 工作线程失败: {e}"))?
}

fn execute(
    mut command: Command,
    limit: Option<Duration>,
    token: CancellationToken,
) -> Result<String, String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn().map_err(|e| format!("无法启动 ADB: {e}"))?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || drain(stdout));
    let err = std::thread::spawn(move || drain(stderr));
    let start = Instant::now();
    let mut failure = None;
    let status = loop {
        if token.is_cancelled() || limit.is_some_and(|d| start.elapsed() >= d) {
            failure = Some(
                if token.is_cancelled() {
                    "ADB 操作已取消"
                } else {
                    "ADB 操作超时，请检查 USB 调试和设备连接"
                }
                .to_string(),
            );
            let _ = child.kill();
            break child.wait();
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e);
            }
        }
    };
    let stdout = String::from_utf8_lossy(&out.join().unwrap_or_default()).to_string();
    let stderr = String::from_utf8_lossy(&err.join().unwrap_or_default()).to_string();
    if let Some(message) = failure {
        return Err(message);
    }
    let status = status.map_err(|e| format!("ADB 进程等待失败: {e}"))?;
    if !status.success() {
        return Err(format!("ADB 操作失败: {}", stderr.trim()));
    }
    Ok(stdout.trim().to_string())
}

pub async fn devices() -> Result<Vec<super::adb::Device>, String> {
    let output = run(
        None,
        vec!["devices".into()],
        Some(Duration::from_secs(3)),
        CancellationToken::new(),
    )
    .await?;
    Ok(output
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let id = parts.next()?;
            let status = parts.next()?;
            if !matches!(
                status,
                "device" | "offline" | "unauthorized" | "recovery" | "sideload" | "bootloader"
            ) {
                return None;
            }
            Some(super::adb::Device {
                id: id.into(),
                status: status.into(),
            })
        })
        .collect())
}

pub async fn cleanup(device: &str, scid: &str) -> Result<(), String> {
    // Only our session's PID may be killed; never pkill all app_process/ADB.
    if scid.len() != 8 || !scid.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("无效投屏会话编号".into());
    }
    let pidfile = format!("/data/local/tmp/jxzs-{scid}.pid");
    let shell = format!(
        r#"
owned() {{
    [ ! -d /proc/$p ] && return 1
    [ -r /proc/$p/cmdline ] || return 2
    c=$(tr '\000' ' ' < /proc/$p/cmdline)
    case "$c" in *com.genymobile.scrcpy.Server*"scid={scid} "*) return 0 ;; *) return 1 ;; esac
}}
if [ -f {pidfile} ]; then
    p=$(cat {pidfile})
    case $p in ''|*[!0-9]*) ;; *)
        owned; result=$?
        [ "$result" != 2 ] || exit 1
        if [ "$result" = 0 ]; then
            kill $p 2>/dev/null
            for i in 1 2 3 4 5; do [ ! -d /proc/$p ] && break; sleep 0.1; done
            owned; result=$?
            [ "$result" != 2 ] || exit 1
            if [ "$result" = 0 ]; then
                kill -9 $p 2>/dev/null
                sleep 0.1
                owned; result=$?
                [ "$result" = 1 ] || exit 1
            fi
        fi
        ;; esac
    rm -f {pidfile}
fi"#
    );
    let remote = run(
        Some(device),
        vec!["shell".into(), shell],
        Some(Duration::from_secs(2)),
        CancellationToken::new(),
    )
    .await;
    let reverse = run(
        Some(device),
        vec![
            "reverse".into(),
            "--remove".into(),
            format!("localabstract:scrcpy_{scid}"),
        ],
        Some(Duration::from_secs(2)),
        CancellationToken::new(),
    )
    .await;
    if let Err(e) = reverse {
        log::debug!("[Projection {scid}] reverse cleanup: {e}");
    }
    remote.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args([
            "--exact",
            "scrcpy::managed_adb::tests::fixture_client",
            "--nocapture",
        ]);
        command.env("JXZS_TEST_ADB_CLIENT", "1");
        command
    }
    #[test]
    fn fixture_client() {
        if std::env::var_os("JXZS_TEST_ADB_CLIENT").is_none() {
            return;
        }
        use std::io::Write;
        let bytes = vec![b'x'; OUTPUT_LIMIT * 4];
        std::io::stdout().write_all(&bytes).unwrap();
        std::io::stderr().write_all(&bytes).unwrap();
        std::thread::sleep(Duration::from_secs(60));
    }
    #[test]
    fn timeout_drains_full_pipes_and_reaps_the_real_client() {
        let start = Instant::now();
        let result = execute(
            fixture(),
            Some(Duration::from_millis(300)),
            CancellationToken::new(),
        );
        assert!(result.unwrap_err().contains("超时"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
    #[test]
    fn stop_terminates_client_without_waiting_for_command_deadline() {
        let token = CancellationToken::new();
        let cancel = token.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            cancel.cancel();
        });
        let start = Instant::now();
        let result = execute(fixture(), Some(Duration::from_secs(60)), token);
        stopper.join().unwrap();
        assert!(result.unwrap_err().contains("取消"));
        assert!(start.elapsed() < Duration::from_secs(5));
    }
}
