//! Serial-device skill — list ports and read/write raw serial I/O. **Off by
//! default** (direct hardware access); gated by `[serial].enabled`. Writes are
//! side-effecting, so `serial_send` goes through the confirmation [`guard`](crate::skills::guard)
//! (first call returns a token; call again with `confirm=<token>`).
//!
//! Blocking serial I/O runs on a blocking thread. Per-call `baud`/`timeout_ms`
//! override the `[serial]` defaults.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::future::BoxFuture;
use rmcp::model::{CallToolResult, JsonObject};
use rmcp::ErrorData as McpError;
use serde::Deserialize;

use crate::skills::guard::Decision;
use crate::skills::{schema_for, NoArgs, Skill, SkillCtx};
use crate::util::truncate_chars;
use crate::{internal, text_result};

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct SendArgs {
    /// Port to open, e.g. `COM3` (Windows) or `/dev/ttyUSB0` (Linux).
    port: String,
    /// Data to write. Sent as UTF-8 bytes (append `\n` yourself if the device needs it).
    data: String,
    /// Baud rate. Omit for the `[serial].baud` default.
    #[serde(default)]
    baud: Option<u32>,
    /// One-time token from a prior call's confirmation prompt. Omit on the first call.
    #[serde(default)]
    confirm: Option<String>,
    /// With `confirm`, stop asking for serial_send for the rest of the session.
    #[serde(default)]
    trust: Option<bool>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
struct ReadArgs {
    /// Port to open, e.g. `COM3` or `/dev/ttyUSB0`.
    port: String,
    /// Baud rate. Omit for the `[serial].baud` default.
    #[serde(default)]
    baud: Option<u32>,
    /// How long to read before returning, in milliseconds. Omit for `[serial].timeout_ms`.
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// Max bytes to read. Default 4096, capped 65536.
    #[serde(default)]
    max_bytes: Option<u32>,
}

pub struct SerialPorts;
impl Skill for SerialPorts {
    fn name(&self) -> &'static str {
        "serial_ports"
    }
    fn description(&self) -> &'static str {
        "List the serial ports available on this machine (name + type, e.g. USB VID/PID). Read-only."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<NoArgs>()
    }
    fn call<'a>(&self, _ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let out = tokio::task::spawn_blocking(list_ports)
                .await
                .map_err(|e| internal(anyhow!("serial task failed: {e}")))?
                .map_err(internal)?;
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[SkillExample {
            title: "Enumerate attached serial ports",
            args: r#"{}"#,
            note: Some("Prints port names + USB VID/PID where available."),
        }]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Discover the COM/tty device path before opening it with `serial_read` or `serial_send`.",
            "Confirm a USB-serial adapter is enumerated by the OS.",
        ]
    }
}

pub struct SerialSend;
impl Skill for SerialSend {
    fn name(&self) -> &'static str {
        "serial_send"
    }
    fn description(&self) -> &'static str {
        "Write data to a serial port (off by default; [serial]). Side-effecting — the first call \
        returns a confirmation token and does nothing; call again with confirm=<token> to send (or \
        confirm + trust=true). Returns the number of bytes written."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<SendArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<SendArgs>()?;
            let summary = format!("write {} byte(s) to {}", args.data.len(), args.port);
            if let Decision::Challenge(msg) = server.guard.check(
                "serial_send",
                "serial_send",
                server.cfg.serial.allow_destructive,
                &summary,
                args.confirm.as_deref(),
                args.trust.unwrap_or(false),
            ) {
                return Ok(text_result(msg));
            }
            let baud = args.baud.unwrap_or(server.serial.baud);
            let timeout = Duration::from_millis(server.serial.timeout_ms.max(1));
            let port = args.port.clone();
            let data = args.data.into_bytes();
            let n = tokio::task::spawn_blocking(move || send(&port, baud, timeout, &data))
                .await
                .map_err(|e| internal(anyhow!("serial task failed: {e}")))?
                .map_err(internal)?;
            Ok(text_result(format!("Wrote {n} byte(s) to {}", args.port)))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "First call returns a confirmation token",
                args: r#"{"port": "COM3", "data": "AT\r\n"}"#,
                note: Some("Returns a token; resend with `confirm=<token>` to actually write."),
            },
            SkillExample {
                title: "Confirmed write at a custom baud",
                args: r#"{"port": "/dev/ttyUSB0", "data": "reset\n", "baud": 9600, "confirm": "abc123"}"#,
                note: None,
            },
            SkillExample {
                title: "Trust the tool for the session",
                args: r#"{"port": "COM3", "data": "ping\n", "confirm": "abc123", "trust": true}"#,
                note: Some(
                    "Subsequent `serial_send` calls skip the challenge until the session ends.",
                ),
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Push an AT command or simple line protocol to a modem / microcontroller.",
            "Trigger an action on a USB-serial device (reset, mode switch) by writing a known command.",
        ]
    }
}

pub struct SerialRead;
impl Skill for SerialRead {
    fn name(&self) -> &'static str {
        "serial_read"
    }
    fn description(&self) -> &'static str {
        "Read from a serial port for up to timeout_ms (or until max_bytes), returning the bytes as \
        text plus a hex dump. Off by default ([serial])."
    }
    fn schema(&self) -> Arc<JsonObject> {
        schema_for::<ReadArgs>()
    }
    fn call<'a>(&self, ctx: SkillCtx<'a>) -> BoxFuture<'a, Result<CallToolResult, McpError>> {
        Box::pin(async move {
            let (server, args) = ctx.parse::<ReadArgs>()?;
            let baud = args.baud.unwrap_or(server.serial.baud);
            let timeout =
                Duration::from_millis(args.timeout_ms.unwrap_or(server.serial.timeout_ms).max(1));
            let max = crate::clamp(args.max_bytes, 4096, 65536);
            let port = args.port.clone();
            let bytes = tokio::task::spawn_blocking(move || read(&port, baud, timeout, max))
                .await
                .map_err(|e| internal(anyhow!("serial task failed: {e}")))?
                .map_err(internal)?;
            if bytes.is_empty() {
                return Ok(text_result(format!(
                    "Read 0 bytes from {} (timeout).",
                    args.port
                )));
            }
            let text = String::from_utf8_lossy(&bytes);
            let hex: String = bytes.iter().map(|b| format!("{b:02x} ")).collect();
            let out = format!(
                "Read {} byte(s) from {}:\n--- text ---\n{}\n--- hex ---\n{}",
                bytes.len(),
                args.port,
                truncate_chars(&text, server.max_chars / 2),
                truncate_chars(hex.trim(), server.max_chars / 2),
            );
            Ok(text_result(out))
        })
    }
    fn examples(&self) -> &'static [crate::skills::SkillExample] {
        use crate::skills::SkillExample;
        &[
            SkillExample {
                title: "Read with defaults",
                args: r#"{"port": "COM3"}"#,
                note: Some(
                    "Uses `[serial].baud` and `[serial].timeout_ms`; returns text + hex dump.",
                ),
            },
            SkillExample {
                title: "Short read with override",
                args: r#"{"port": "/dev/ttyUSB0", "baud": 115200, "timeout_ms": 500, "max_bytes": 256}"#,
                note: None,
            },
        ]
    }
    fn use_cases(&self) -> &'static [&'static str] {
        &[
            "Capture the response banner / output of a device right after opening the port.",
            "Read the reply to a `serial_send` command and inspect it as both text and hex.",
        ]
    }
}

// --- blocking serial primitives (run on spawn_blocking) ---------------------

fn list_ports() -> Result<String> {
    let ports = serialport::available_ports()
        .map_err(|e| anyhow!("could not enumerate serial ports: {e}"))?;
    if ports.is_empty() {
        return Ok("No serial ports found.".into());
    }
    let mut out = format!("Serial ports ({}):\n", ports.len());
    for p in &ports {
        let kind = match &p.port_type {
            serialport::SerialPortType::UsbPort(u) => format!(
                "USB {:04x}:{:04x}{}",
                u.vid,
                u.pid,
                u.product
                    .as_deref()
                    .map(|s| format!(" ({s})"))
                    .unwrap_or_default()
            ),
            serialport::SerialPortType::BluetoothPort => "Bluetooth".into(),
            serialport::SerialPortType::PciPort => "PCI".into(),
            serialport::SerialPortType::Unknown => "unknown".into(),
        };
        out.push_str(&format!("  {}  [{kind}]\n", p.port_name));
    }
    Ok(out)
}

fn open(port: &str, baud: u32, timeout: Duration) -> Result<Box<dyn serialport::SerialPort>> {
    serialport::new(port, baud)
        .timeout(timeout)
        .open()
        .map_err(|e| anyhow!("could not open '{port}' at {baud} baud: {e}"))
}

fn send(port: &str, baud: u32, timeout: Duration, data: &[u8]) -> Result<usize> {
    use std::io::Write;
    let mut p = open(port, baud, timeout)?;
    p.write_all(data)
        .map_err(|e| anyhow!("write to '{port}' failed: {e}"))?;
    p.flush().ok();
    Ok(data.len())
}

fn read(port: &str, baud: u32, timeout: Duration, max: usize) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut p = open(port, baud, timeout)?;
    let mut out = Vec::new();
    let mut buf = [0u8; 1024];
    // Read until the timeout elapses on an empty read, or max bytes reached.
    loop {
        match p.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                out.extend_from_slice(&buf[..n]);
                if out.len() >= max {
                    out.truncate(max);
                    break;
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::TimedOut => break,
            Err(e) => return Err(anyhow!("read from '{port}' failed: {e}")),
        }
    }
    Ok(out)
}

pub struct Family;
impl crate::skills::FamilyMeta for Family {
    fn family(&self) -> &'static str {
        "serial"
    }
    fn tools(&self) -> Vec<&'static str> {
        skills().iter().map(|s| s.name()).collect()
    }
    fn description(&self) -> &'static str {
        "List, open, and (with confirmation) write to local serial ports — UART/USB-serial \
         devices for microcontrollers, sensors, modems. Off by default; requires the host \
         to actually expose serial devices the user account can access."
    }
    fn check_capability(&self) -> crate::skills::SkillCapability {
        use crate::skills::SkillCapability;
        // The serialport crate enumerates ports via the OS — an empty
        // list is a legitimate "no devices attached", which we still
        // report as Ready (the LLM might be expected to wait for a USB
        // plug). We only flag unavailable when the enumeration itself
        // fails (e.g. missing udev/libudev on a Linux container).
        match serialport::available_ports() {
            Ok(_) => SkillCapability::Ready,
            Err(e) => SkillCapability::unavailable(
                format!("serial port enumeration failed: {e}"),
                "Linux containers need libudev1; the host needs a serial device subsystem",
            ),
        }
    }
}

/// The skills this module contributes (gating happens in `disabled_by_config`).
pub fn skills() -> Vec<Box<dyn Skill>> {
    vec![
        Box::new(SerialPorts),
        Box::new(SerialSend),
        Box::new(SerialRead),
    ]
}
