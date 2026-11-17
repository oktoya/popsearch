use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter};

pub const SYSTEM_PROMPT: &str = "You are PopSearch, a fast answer assistant opened from a desktop search bar. Answer in the language of the user's question. Be concise and direct; lead with the answer. Use Markdown when it helps (lists, tables, code blocks). Code blocks must always state the language. If the content of a code block itself contains a line of three backticks, fence that block with four or more backticks (one more than the longest backtick run inside).";

pub const NOT_FOUND_MSG: &str =
    "Claude Code CLI를 찾을 수 없습니다. claude 명령을 설치하거나 POPSEARCH_CLAUDE_BIN을 지정하세요.";

#[derive(Debug, PartialEq)]
pub enum CliEvent {
    Chunk(String),
    Done,
    Error(String),
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AttachmentInput {
    pub kind: String,
    pub media_type: Option<String>,
    pub data: Option<String>,
    pub text: Option<String>,
    pub name: String,
}

pub fn build_message_line(text: &str, attachments: &[AttachmentInput]) -> String {
    let content = if attachments.is_empty() {
        serde_json::Value::String(text.to_string())
    } else {
        let mut blocks: Vec<serde_json::Value> = Vec::new();
        for a in attachments.iter().filter(|a| a.kind == "image") {
            blocks.push(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": a.media_type.clone().unwrap_or_default(),
                    "data": a.data.clone().unwrap_or_default()
                }
            }));
        }
        for a in attachments.iter() {
            if a.kind == "pdf" {
                blocks.push(serde_json::json!({
                    "type": "document",
                    "source": {
                        "type": "base64",
                        "media_type": "application/pdf",
                        "data": a.data.clone().unwrap_or_default()
                    },
                    "title": a.name
                }));
            } else if a.kind == "text" {
                blocks.push(serde_json::json!({
                    "type": "document",
                    "source": {
                        "type": "text",
                        "media_type": "text/plain",
                        "data": a.text.clone().unwrap_or_default()
                    },
                    "title": a.name
                }));
            }
        }
        if !text.trim().is_empty() {
            blocks.push(serde_json::json!({ "type": "text", "text": text }));
        }
        serde_json::Value::Array(blocks)
    };
    serde_json::json!({
        "type": "user",
        "message": { "role": "user", "content": content }
    })
    .to_string()
}

pub fn parse_line(line: &str) -> Option<CliEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v.get("type")?.as_str()? {
        "stream_event" => {
            let ev = v.get("event")?;
            if ev.get("type")?.as_str()? != "content_block_delta" {
                return None;
            }
            let delta = ev.get("delta")?;
            if delta.get("type")?.as_str()? != "text_delta" {
                return None;
            }
            Some(CliEvent::Chunk(delta.get("text")?.as_str()?.to_string()))
        }
        "result" => {
            if v.get("is_error").and_then(|b| b.as_bool()) == Some(true) {
                let msg = v
                    .get("result")
                    .and_then(|r| r.as_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("CLI 오류가 발생했습니다")
                    .to_string();
                Some(CliEvent::Error(msg))
            } else {
                Some(CliEvent::Done)
            }
        }
        _ => None,
    }
}

fn is_file(p: &Path) -> bool {
    p.is_file()
}

pub fn find_claude_bin() -> Option<PathBuf> {
    if let Some(v) = std::env::var_os("POPSEARCH_CLAUDE_BIN") {
        let p = PathBuf::from(v);
        if is_file(&p) {
            return Some(p);
        }
    }
    let path = std::env::var_os("PATH")?;
    let dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    if cfg!(windows) {
        for d in &dirs {
            let p = d.join("claude.exe");
            if is_file(&p) {
                return Some(p);
            }
        }
        for d in &dirs {
            if is_file(&d.join("claude.cmd")) {
                let p = d
                    .join("node_modules")
                    .join("@anthropic-ai")
                    .join("claude-code")
                    .join("bin")
                    .join("claude.exe");
                if is_file(&p) {
                    return Some(p);
                }
            }
        }
        None
    } else {
        dirs.iter().map(|d| d.join("claude")).find(|p| is_file(p))
    }
}

pub fn normalize_model(model: &str) -> &'static str {
    match model {
        "sonnet" => "sonnet",
        "opus" => "opus",
        _ => "haiku",
    }
}

fn effort_for(model: &str) -> &'static str {
    if model == "haiku" {
        "low"
    } else {
        "medium"
    }
}

pub struct CliSession {
    child: Child,
    stdin: Option<ChildStdin>,
    in_flight: Arc<AtomicBool>,
    killed: Arc<AtomicBool>,
    exited: Arc<AtomicBool>,
}

impl CliSession {
    pub fn spawn(app: &AppHandle, model: &str) -> Result<CliSession, String> {
        let bin = find_claude_bin().ok_or_else(|| NOT_FOUND_MSG.to_string())?;
        let mut cmd = Command::new(bin);
        cmd.args([
            "-p",
            "--model",
            model,
            "--effort",
            effort_for(model),
            "--tools",
            "",
            "--strict-mcp-config",
            "--setting-sources",
            "",
            "--disable-slash-commands",
            "--no-session-persistence",
            "--system-prompt",
            SYSTEM_PROMPT,
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--include-partial-messages",
            "--verbose",
        ]);
        cmd.current_dir(std::env::temp_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000);
        }
        let mut child = cmd.spawn().map_err(|e| format!("CLI 실행 실패: {e}"))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let in_flight = Arc::new(AtomicBool::new(false));
        let killed = Arc::new(AtomicBool::new(false));
        let exited = Arc::new(AtomicBool::new(false));

        if let Some(stderr) = stderr {
            std::thread::spawn(move || {
                let mut r = BufReader::new(stderr);
                let mut buf = String::new();
                while matches!(r.read_line(&mut buf), Ok(n) if n > 0) {
                    buf.clear();
                }
            });
        }

        if let Some(stdout) = stdout {
            let app = app.clone();
            let in_flight = in_flight.clone();
            let killed = killed.clone();
            let exited = exited.clone();
            std::thread::spawn(move || {
                let r = BufReader::new(stdout);
                for line in r.lines() {
                    let Ok(line) = line else { break };
                    match parse_line(&line) {
                        Some(CliEvent::Chunk(text)) => {
                            if killed.load(Ordering::SeqCst) {
                                continue;
                            }
                            let _ = app.emit("popsearch://chunk", serde_json::json!({ "text": text }));
                        }
                        Some(CliEvent::Done) => {
                            in_flight.store(false, Ordering::SeqCst);
                            if !killed.load(Ordering::SeqCst) {
                                let _ = app.emit("popsearch://done", serde_json::json!({}));
                            }
                        }
                        Some(CliEvent::Error(message)) => {
                            in_flight.store(false, Ordering::SeqCst);
                            if !killed.load(Ordering::SeqCst) {
                                let _ = app.emit(
                                    "popsearch://error",
                                    serde_json::json!({ "message": message }),
                                );
                            }
                        }
                        None => {}
                    }
                }
                exited.store(true, Ordering::SeqCst);
                if in_flight.swap(false, Ordering::SeqCst) && !killed.load(Ordering::SeqCst) {
                    let _ = app.emit(
                        "popsearch://error",
                        serde_json::json!({ "message": "CLI 프로세스가 종료되었습니다" }),
                    );
                }
            });
        } else {
            exited.store(true, Ordering::SeqCst);
        }

        Ok(CliSession {
            child,
            stdin,
            in_flight,
            killed,
            exited,
        })
    }

    pub fn is_alive(&mut self) -> bool {
        if self.exited.load(Ordering::SeqCst) {
            return false;
        }
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn send(&mut self, text: &str, attachments: &[AttachmentInput]) -> Result<(), String> {
        let line = build_message_line(text, attachments);
        let stdin = self.stdin.as_mut().ok_or("CLI 입력 파이프가 없습니다")?;
        self.in_flight.store(true, Ordering::SeqCst);
        let res = writeln!(stdin, "{line}").and_then(|_| stdin.flush());
        if let Err(e) = res {
            self.in_flight.store(false, Ordering::SeqCst);
            return Err(format!("CLI에 질문을 보내지 못했습니다: {e}"));
        }
        Ok(())
    }

    pub fn kill(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
        self.stdin = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Default)]
struct Sessions {
    current: HashMap<&'static str, CliSession>,
    spare: Option<CliSession>,
    spare_model: &'static str,
}

pub struct Manager {
    inner: Arc<Mutex<Sessions>>,
}

fn default_model(app: &AppHandle) -> &'static str {
    normalize_model(&crate::settings::get_settings(app.clone()).default_model)
}

impl Manager {
    pub fn new() -> Manager {
        Manager {
            inner: Arc::new(Mutex::new(Sessions::default())),
        }
    }

    /// Spawn the default-model current session and spare in the background (does not block).
    pub fn warm_up(&self, app: &AppHandle) {
        let inner = self.inner.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            let model = default_model(&app);
            if let Ok(s) = CliSession::spawn(&app, model) {
                let mut g = inner.lock().unwrap();
                if !g.current.contains_key(model) {
                    g.current.insert(model, s);
                } else {
                    g.spare = Some(s);
                    g.spare_model = model;
                }
            }
            if let Ok(s) = CliSession::spawn(&app, model) {
                let mut g = inner.lock().unwrap();
                if g.spare.is_none() {
                    g.spare = Some(s);
                    g.spare_model = model;
                } else {
                    let mut s = s;
                    s.kill();
                }
            }
        });
    }

    fn spawn_spare_background(&self, app: &AppHandle) {
        let inner = self.inner.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            let model = default_model(&app);
            if let Ok(s) = CliSession::spawn(&app, model) {
                let mut g = inner.lock().unwrap();
                if g.spare.is_none() {
                    g.spare = Some(s);
                    g.spare_model = model;
                } else {
                    let mut s = s;
                    s.kill();
                }
            }
        });
    }

    pub fn ask(&self, app: &AppHandle, model: &str, text: &str, attachments: &[AttachmentInput]) {
        let model = normalize_model(model);
        let result = (|| -> Result<(), String> {
            let mut g = self.inner.lock().unwrap();
            let alive = g.current.get_mut(model).map(|s| s.is_alive()).unwrap_or(false);
            if !alive {
                if let Some(mut old) = g.current.remove(model) {
                    old.kill();
                }
                g.current.insert(model, CliSession::spawn(app, model)?);
            }
            g.current.get_mut(model).unwrap().send(text, attachments)
        })();
        if let Err(message) = result {
            let _ = app.emit("popsearch://error", serde_json::json!({ "message": message }));
        }
    }

    pub fn new_conversation(&self, app: &AppHandle) {
        {
            let mut g = self.inner.lock().unwrap();
            for (_, mut old) in g.current.drain() {
                old.kill();
            }
            let model = default_model(app);
            if let Some(mut s) = g.spare.take() {
                if g.spare_model == model {
                    g.current.insert(model, s);
                } else {
                    s.kill();
                }
            }
        }
        self.spawn_spare_background(app);
    }

    pub fn kill_all(&self) {
        let mut g = self.inner.lock().unwrap();
        for (_, mut s) in g.current.drain() {
            s.kill();
        }
        if let Some(mut s) = g.spare.take() {
            s.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_delta() {
        let l = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"안녕"}}}"#;
        assert_eq!(parse_line(l), Some(CliEvent::Chunk("안녕".to_string())));
    }

    #[test]
    fn non_text_delta_ignored() {
        let l = r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}}"#;
        assert_eq!(parse_line(l), None);
    }

    #[test]
    fn result_success() {
        let l = r#"{"type":"result","subtype":"success","is_error":false,"result":"ok"}"#;
        assert_eq!(parse_line(l), Some(CliEvent::Done));
    }

    #[test]
    fn result_error() {
        let l = r#"{"type":"result","subtype":"error","is_error":true,"result":"boom"}"#;
        assert_eq!(parse_line(l), Some(CliEvent::Error("boom".to_string())));
    }

    fn att(kind: &str) -> AttachmentInput {
        AttachmentInput {
            kind: kind.to_string(),
            media_type: None,
            data: None,
            text: None,
            name: "f".to_string(),
        }
    }

    #[test]
    fn message_line_with_image() {
        let imgs = vec![AttachmentInput {
            media_type: Some("image/png".to_string()),
            data: Some("AAAA".to_string()),
            ..att("image")
        }];
        let v: serde_json::Value = serde_json::from_str(&build_message_line("hi", &imgs)).unwrap();
        let c = v["message"]["content"].as_array().unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0]["type"], "image");
        assert_eq!(c[0]["source"]["type"], "base64");
        assert_eq!(c[0]["source"]["media_type"], "image/png");
        assert_eq!(c[0]["source"]["data"], "AAAA");
        assert_eq!(c[1]["type"], "text");
        assert_eq!(c[1]["text"], "hi");
    }

    #[test]
    fn message_line_with_pdf() {
        let a = vec![AttachmentInput {
            media_type: Some("application/pdf".to_string()),
            data: Some("JVBER".to_string()),
            name: "a.pdf".to_string(),
            ..att("pdf")
        }];
        let v: serde_json::Value = serde_json::from_str(&build_message_line("hi", &a)).unwrap();
        let c = v["message"]["content"].as_array().unwrap();
        assert_eq!(c.len(), 2);
        assert_eq!(c[0]["type"], "document");
        assert_eq!(c[0]["source"]["type"], "base64");
        assert_eq!(c[0]["source"]["media_type"], "application/pdf");
        assert_eq!(c[0]["source"]["data"], "JVBER");
        assert_eq!(c[0]["title"], "a.pdf");
        assert_eq!(c[1]["type"], "text");
    }

    #[test]
    fn message_line_with_text_doc() {
        let a = vec![AttachmentInput {
            text: Some("hello".to_string()),
            name: "a.txt".to_string(),
            ..att("text")
        }];
        let v: serde_json::Value = serde_json::from_str(&build_message_line("", &a)).unwrap();
        let c = v["message"]["content"].as_array().unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0]["type"], "document");
        assert_eq!(c[0]["source"]["type"], "text");
        assert_eq!(c[0]["source"]["media_type"], "text/plain");
        assert_eq!(c[0]["source"]["data"], "hello");
        assert_eq!(c[0]["title"], "a.txt");
    }

    #[test]
    fn message_line_without_image() {
        let v: serde_json::Value = serde_json::from_str(&build_message_line("hi", &[])).unwrap();
        assert_eq!(v["message"]["content"], "hi");
    }

    #[test]
    fn model_and_effort() {
        assert_eq!(normalize_model("sonnet"), "sonnet");
        assert_eq!(normalize_model("opus"), "opus");
        assert_eq!(normalize_model("haiku"), "haiku");
        assert_eq!(normalize_model("gpt"), "haiku");
        assert_eq!(effort_for("haiku"), "low");
        assert_eq!(effort_for("sonnet"), "medium");
        assert_eq!(effort_for("opus"), "medium");
    }

    #[test]
    fn invalid_json_ignored() {
        assert_eq!(parse_line("not json"), None);
    }
}
