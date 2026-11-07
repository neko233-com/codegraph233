//! Use PowerShell 7's official parser when the portable grammar reports recovery.
//! Fixed trusted code receives source as JSON on stdin; source is never invoked.
use crate::model::FileParse;
use base64::{engine::general_purpose::STANDARD, Engine};
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn parse(rel: &str, root: &str, module: &str, source: &[u8]) -> Result<FileParse, String> {
    let source = std::str::from_utf8(source).map_err(|_| "PowerShell source is not UTF-8")?;
    let helper = include_str!("../tools/powershell/Export-Ast.ps1");
    let encoded = STANDARD.encode(
        helper
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let executable = std::env::var("CODEGRAPH233_PWSH").unwrap_or_else(|_| "pwsh".into());
    let mut command = Command::new(executable);
    command.args([
        "-NoLogo",
        "-NoProfile",
        "-NonInteractive",
        "-EncodedCommand",
        &encoded,
    ]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("PowerShell 7 parser unavailable: {e}"))?;
    let payload =
        serde_json::json!({"source":source,"rel":rel,"root":root,"module":module}).to_string();
    let mut stdin = child.stdin.take().ok_or("missing parser stdin")?;
    let writer = std::thread::spawn(move || stdin.write_all(payload.as_bytes()));
    let stdout = child.stdout.take().ok_or("missing parser stdout")?;
    let stderr = child.stderr.take().ok_or("missing parser stderr")?;
    let out = std::thread::spawn(move || {
        let mut data = Vec::new();
        stdout
            .take(32 * 1024 * 1024 + 1)
            .read_to_end(&mut data)
            .map(|_| data)
    });
    let err = std::thread::spawn(move || {
        let mut data = Vec::new();
        stderr.take(65537).read_to_end(&mut data).map(|_| data)
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("PowerShell parser exceeded 10-second deadline".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let written = writer.join().map_err(|_| "parser input thread failed")?;
    let stdout = out
        .join()
        .map_err(|_| "parser output thread failed")?
        .map_err(|e| e.to_string())?;
    let stderr = err
        .join()
        .map_err(|_| "parser error thread failed")?
        .map_err(|e| e.to_string())?;
    if written.is_err()
        || !status.success()
        || stdout.len() > 32 * 1024 * 1024
        || stderr.len() > 65536
    {
        return Err(format!(
            "PowerShell parser failed (exit={status}; stderr={} bytes)",
            stderr.len()
        ));
    }
    let value: serde_json::Value =
        serde_json::from_slice(&stdout).map_err(|e| format!("PowerShell parser JSON: {e}"))?;
    if value["format"] != "codegraph233.powershell-ast/v1" {
        return Err("PowerShell parser format differs".into());
    }
    let version = value["parser_version"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();
    let mut parsed: FileParse = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if parsed.rel_path != rel
        || parsed.root != root
        || parsed.lang != "powershell"
        || parsed.module != module
    {
        return Err("PowerShell parser input identity differs".into());
    }
    for symbol in &mut parsed.symbols {
        symbol.modifiers = format!("PowerShell native parser {version}; source was not executed");
    }
    Ok(parsed)
}
