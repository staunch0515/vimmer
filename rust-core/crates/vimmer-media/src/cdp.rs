//! A minimal Chrome DevTools Protocol client over `--remote-debugging-pipe`, just enough to
//! screenshot one page.
//!
//! Chrome reads commands from fd 3 and writes responses and events to fd 4, each message one
//! JSON object terminated by a NUL byte, so no websocket is needed. This is how Remotion (via
//! puppeteer) drives the same `chrome-headless-shell`, and it works on machines where the
//! `--screenshot` command-line mode never finishes (seen on an Apple Silicon iMac with macOS
//! 26, where headless Chrome's `--screenshot` waits forever for a frame that never comes).
//!
//! The conversation: `Target.createTarget` + `Target.attachToTarget` (flat session),
//! `Page.enable`, `Emulation.setDeviceMetricsOverride` (exact canvas size, scale 1),
//! `Page.navigate` and wait for `Page.loadEventFired`, `Page.captureScreenshot` (PNG). Chrome
//! runs in its own process group, which is killed afterwards, on timeout and on cancellation.

use std::ffi::c_int;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::Limits;

/// How often the deadline and the cancellation check are looked at while waiting for Chrome.
const POLL: Duration = Duration::from_millis(50);
/// Largest single message accepted from Chrome (a 4K PNG in base64 is well below this).
const MAX_MESSAGE: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum CdpError {
    Cancelled,
    TimedOut { secs: u64 },
    Failed(String),
}

unsafe extern "C" {
    fn dup2(old: c_int, new: c_int) -> c_int;
    fn fcntl(fd: c_int, cmd: c_int, ...) -> c_int;
    fn close(fd: c_int) -> c_int;
}

/// `F_DUPFD` is 0 on macOS and Linux.
const F_DUPFD: c_int = 0;

/// Loads `url` in headless `chrome` with a `width` x `height` viewport and returns the PNG.
/// `args` are the Chrome switches (proxy, window size, ...); `--remote-debugging-pipe` and the
/// start page are added here. Chrome runs with `work_dir` (the scratch profile dir, created if
/// needed) as its working directory, so nothing it might write lands anywhere else.
pub(crate) fn capture_png(
    chrome: &Path,
    args: &[String],
    work_dir: &Path,
    url: &str,
    width: u32,
    height: u32,
    limits: &Limits,
) -> Result<Vec<u8>, CdpError> {
    if (limits.cancelled)() {
        return Err(CdpError::Cancelled);
    }
    let started = Instant::now();
    let fail = |what: &str, e: std::io::Error| CdpError::Failed(format!("{what}: {e}"));
    std::fs::create_dir_all(work_dir).map_err(|e| fail("cannot create the profile dir", e))?;
    // Commands: we write `to_chrome`, Chrome reads `chrome_in` as fd 3.
    let (chrome_in, mut to_chrome) = std::io::pipe().map_err(|e| fail("pipe", e))?;
    // Responses: Chrome writes `chrome_out` as fd 4, we read `from_chrome`.
    let (from_chrome, chrome_out) = std::io::pipe().map_err(|e| fail("pipe", e))?;
    let (in_fd, out_fd) = (chrome_in.as_raw_fd(), chrome_out.as_raw_fd());

    let mut cmd = Command::new(chrome);
    cmd.args(args)
        .arg("--remote-debugging-pipe")
        .arg("about:blank")
        .current_dir(work_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .process_group(0);
    // SAFETY: only async-signal-safe calls (fcntl, dup2, close) between fork and exec. The pipe
    // ends are first copied above fd 10, so placing them on 3 and 4 cannot clobber one with the
    // other; dup2 clears close-on-exec on the copies Chrome keeps.
    unsafe {
        cmd.pre_exec(move || {
            let a = fcntl(in_fd, F_DUPFD, 10 as c_int);
            let b = fcntl(out_fd, F_DUPFD, 10 as c_int);
            if a < 0 || b < 0 || dup2(a, 3) < 0 || dup2(b, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            close(a);
            close(b);
            Ok(())
        });
    }
    let child = cmd
        .spawn()
        .map_err(|e| CdpError::Failed(format!("cannot run {}: {e}", chrome.display())))?;
    // Only Chrome may hold these ends, or its exit would never show up as end-of-file here.
    drop(chrome_in);
    drop(chrome_out);
    let mut chrome = Running::new(child, from_chrome);

    let deadline = limits.timeout.map(|t| (started + t, t.as_secs()));
    let mut session = Session {
        next_id: 0,
        to_chrome: &mut to_chrome,
        chrome: &mut chrome,
        limits,
        deadline,
    };
    let result = session.screenshot(url, width, height);
    drop(to_chrome);
    let stderr = chrome.finish();
    result.map_err(|e| match e {
        CdpError::Failed(m) => {
            let last = stderr
                .lines()
                .rev()
                .find(|l| !l.trim().is_empty())
                .map(|l| format!(" (Chrome said: {})", l.trim()))
                .unwrap_or_default();
            CdpError::Failed(format!("{m}{last}"))
        }
        other => other,
    })
}

/// The Chrome child with its message and stderr readers.
struct Running {
    child: Child,
    messages: Receiver<Result<Vec<u8>, String>>,
    reader: Option<JoinHandle<()>>,
    stderr: Option<JoinHandle<Vec<u8>>>,
}

impl Running {
    fn new(mut child: Child, from_chrome: std::io::PipeReader) -> Self {
        let (tx, messages) = mpsc::channel();
        let reader = thread::spawn(move || read_messages(from_chrome, &tx));
        let stderr = child.stderr.take().map(|mut pipe| {
            thread::spawn(move || {
                let mut buf = Vec::new();
                let _ = pipe.read_to_end(&mut buf);
                buf
            })
        });
        Self {
            child,
            messages,
            reader: Some(reader),
            stderr,
        }
    }

    /// Kills the whole process group (the PNG is already in hand, so there is nothing to wait
    /// for; Chrome does not exit promptly after `Browser.close` anyway) and collects stderr.
    fn finish(&mut self) -> String {
        kill_group(self.child.id());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        self.stderr
            .take()
            .and_then(|h| h.join().ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
            .unwrap_or_default()
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if self.reader.is_some() {
            self.finish();
        }
    }
}

/// SIGKILL to the process group led by `pid`.
fn kill_group(pid: u32) {
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{pid}")])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

/// Splits Chrome's output into NUL-terminated messages until end-of-file.
fn read_messages(mut pipe: impl Read, tx: &mpsc::Sender<Result<Vec<u8>, String>>) {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 256 * 1024];
    loop {
        let n = match pipe.read(&mut chunk) {
            Ok(0) => return,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                let _ = tx.send(Err(e.to_string()));
                return;
            }
        };
        let start = buf.len();
        buf.extend_from_slice(&chunk[..n]);
        let mut from = 0;
        let mut search = start;
        while let Some(pos) = buf[search..].iter().position(|&b| b == 0) {
            let end = search + pos;
            if tx.send(Ok(buf[from..end].to_vec())).is_err() {
                return;
            }
            from = end + 1;
            search = from;
        }
        buf.drain(..from);
        if buf.len() > MAX_MESSAGE {
            let _ = tx.send(Err("a DevTools message is too large".into()));
            return;
        }
    }
}

struct Session<'a> {
    next_id: u64,
    to_chrome: &'a mut std::io::PipeWriter,
    chrome: &'a mut Running,
    limits: &'a Limits,
    deadline: Option<(Instant, u64)>,
}

impl Session<'_> {
    fn screenshot(&mut self, url: &str, width: u32, height: u32) -> Result<Vec<u8>, CdpError> {
        let target = self.call("Target.createTarget", r#"{"url":"about:blank"}"#, None)?;
        let target_id = target
            .get("targetId")
            .and_then(Json::as_str)
            .ok_or_else(|| CdpError::Failed("Target.createTarget returned no targetId".into()))?
            .to_string();
        let attached = self.call(
            "Target.attachToTarget",
            &format!(
                r#"{{"targetId":{},"flatten":true}}"#,
                json_string(&target_id)
            ),
            None,
        )?;
        let session_id = attached
            .get("sessionId")
            .and_then(Json::as_str)
            .ok_or_else(|| CdpError::Failed("Target.attachToTarget returned no sessionId".into()))?
            .to_string();
        let sid = Some(session_id.as_str());
        self.call("Page.enable", "{}", sid)?;
        self.call(
            "Emulation.setDeviceMetricsOverride",
            &format!(
                r#"{{"width":{width},"height":{height},"deviceScaleFactor":1,"mobile":false}}"#
            ),
            sid,
        )?;
        // The load event can arrive before or after the navigate response.
        let nav_id = self.send(
            "Page.navigate",
            &format!(r#"{{"url":{}}}"#, json_string(url)),
            sid,
        )?;
        let (mut navigated, mut loaded) = (false, false);
        while !(navigated && loaded) {
            let msg = self.next_message()?;
            if msg.get("id").and_then(Json::as_u64) == Some(nav_id) {
                let result = response_result(&msg, "Page.navigate")?;
                if let Some(err) = result.get("errorText").and_then(Json::as_str) {
                    return Err(CdpError::Failed(format!("navigation failed: {err}")));
                }
                navigated = true;
            } else if msg.get("sessionId").and_then(Json::as_str) == sid {
                match msg.get("method").and_then(Json::as_str) {
                    Some("Page.loadEventFired") => loaded = true,
                    Some("Inspector.targetCrashed") => {
                        return Err(CdpError::Failed("the page crashed".into()))
                    }
                    _ => {}
                }
            }
        }
        let shot = self.call("Page.captureScreenshot", r#"{"format":"png"}"#, sid)?;
        let data = shot
            .get("data")
            .and_then(Json::as_str)
            .ok_or_else(|| CdpError::Failed("Page.captureScreenshot returned no data".into()))?;
        base64_decode(data)
            .ok_or_else(|| CdpError::Failed("Page.captureScreenshot returned bad base64".into()))
    }

    fn send(&mut self, method: &str, params: &str, session: Option<&str>) -> Result<u64, CdpError> {
        self.next_id += 1;
        let id = self.next_id;
        let session = session
            .map(|s| format!(r#","sessionId":{}"#, json_string(s)))
            .unwrap_or_default();
        let msg = format!(r#"{{"id":{id},"method":"{method}","params":{params}{session}}}"#);
        let mut bytes = msg.into_bytes();
        bytes.push(0);
        self.to_chrome
            .write_all(&bytes)
            .and_then(|()| self.to_chrome.flush())
            .map_err(|e| CdpError::Failed(format!("cannot send {method} to Chrome: {e}")))?;
        Ok(id)
    }

    /// Sends a command and waits for its response, skipping events.
    fn call(
        &mut self,
        method: &str,
        params: &str,
        session: Option<&str>,
    ) -> Result<Json, CdpError> {
        let id = self.send(method, params, session)?;
        loop {
            let msg = self.next_message()?;
            if msg.get("id").and_then(Json::as_u64) == Some(id) {
                return response_result(&msg, method).cloned();
            }
        }
    }

    fn next_message(&mut self) -> Result<Json, CdpError> {
        loop {
            if (self.limits.cancelled)() {
                return Err(CdpError::Cancelled);
            }
            if let Some((at, secs)) = self.deadline {
                if Instant::now() >= at {
                    return Err(CdpError::TimedOut { secs });
                }
            }
            match self.chrome.messages.recv_timeout(POLL) {
                Ok(Ok(bytes)) => {
                    let text = String::from_utf8_lossy(&bytes);
                    return Json::parse(&text).ok_or_else(|| {
                        CdpError::Failed("Chrome sent a message that is not JSON".into())
                    });
                }
                Ok(Err(e)) => return Err(CdpError::Failed(format!("reading from Chrome: {e}"))),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    let status = self
                        .chrome
                        .child
                        .try_wait()
                        .ok()
                        .flatten()
                        .map(|s| format!(" ({s})"))
                        .unwrap_or_default();
                    return Err(CdpError::Failed(format!(
                        "Chrome closed the DevTools pipe{status}"
                    )));
                }
            }
        }
    }
}

fn response_result<'a>(msg: &'a Json, method: &str) -> Result<&'a Json, CdpError> {
    if let Some(err) = msg.get("error") {
        let text = err
            .get("message")
            .and_then(Json::as_str)
            .unwrap_or("unknown error");
        return Err(CdpError::Failed(format!("{method}: {text}")));
    }
    msg.get("result")
        .ok_or_else(|| CdpError::Failed(format!("{method}: response without result")))
}

/// `s` as a JSON string literal.
pub(crate) fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Standard base64 (with or without padding); `None` on any other character.
pub(crate) fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0u32);
    for &b in s.trim_end_matches('=').as_bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        acc = (acc << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// A parsed JSON value, enough for DevTools responses.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    pub(crate) fn parse(text: &str) -> Option<Json> {
        let mut p = JsonParser {
            s: text.as_bytes(),
            i: 0,
        };
        let v = p.value(0)?;
        p.ws();
        (p.i == p.s.len()).then_some(v)
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Num(n) if *n >= 0.0 && n.fract() == 0.0 => Some(*n as u64),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    s: &'a [u8],
    i: usize,
}

impl JsonParser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > 64 {
            return None;
        }
        self.ws();
        match *self.s.get(self.i)? {
            b'{' => {
                self.i += 1;
                let mut fields = Vec::new();
                self.ws();
                if self.eat(b"}") {
                    return Some(Json::Obj(fields));
                }
                loop {
                    self.ws();
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(b":") {
                        return None;
                    }
                    fields.push((key, self.value(depth + 1)?));
                    self.ws();
                    if self.eat(b"}") {
                        return Some(Json::Obj(fields));
                    }
                    if !self.eat(b",") {
                        return None;
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.eat(b"]") {
                    return Some(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.ws();
                    if self.eat(b"]") {
                        return Some(Json::Arr(items));
                    }
                    if !self.eat(b",") {
                        return None;
                    }
                }
            }
            b'"' => self.string().map(Json::Str),
            b't' => self.eat(b"true").then_some(Json::Bool(true)),
            b'f' => self.eat(b"false").then_some(Json::Bool(false)),
            b'n' => self.eat(b"null").then_some(Json::Null),
            _ => {
                let start = self.i;
                while self
                    .s
                    .get(self.i)
                    .is_some_and(|b| b"+-.eE0123456789".contains(b))
                {
                    self.i += 1;
                }
                std::str::from_utf8(&self.s[start..self.i])
                    .ok()?
                    .parse()
                    .ok()
                    .map(Json::Num)
            }
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let h = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
        self.i += 4;
        u32::from_str_radix(h, 16).ok()
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat(b"\"") {
            return None;
        }
        let mut out = String::new();
        loop {
            // Copy the run up to the next quote or backslash in one go.
            let start = self.i;
            while self.s.get(self.i).is_some_and(|&b| b != b'"' && b != b'\\') {
                self.i += 1;
            }
            out.push_str(std::str::from_utf8(&self.s[start..self.i]).ok()?);
            match *self.s.get(self.i)? {
                b'"' => {
                    self.i += 1;
                    return Some(out);
                }
                _ => {
                    self.i += 1;
                    let esc = *self.s.get(self.i)?;
                    self.i += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let hi = self.hex4()?;
                            let c = if (0xd800..0xdc00).contains(&hi) && self.eat(b"\\u") {
                                let lo = self.hex4()?;
                                0x10000 + ((hi - 0xd800) << 10) + (lo.wrapping_sub(0xdc00) & 0x3ff)
                            } else {
                                hi
                            };
                            out.push(char::from_u32(c).unwrap_or('\u{fffd}'));
                        }
                        _ => return None,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("vimmer-cdp-test-{name}-{}", std::process::id()))
    }

    #[test]
    fn parses_devtools_messages() {
        let m = Json::parse(
            r#"{"id":3,"result":{"data":"iVBO\/w==","n":-1.5e2,"ok":true,"x":null,"a":[1,"é😀"]}}"#,
        )
        .unwrap();
        assert_eq!(m.get("id").and_then(Json::as_u64), Some(3));
        let r = m.get("result").unwrap();
        assert_eq!(r.get("data").and_then(Json::as_str), Some("iVBO/w=="));
        assert_eq!(r.get("n"), Some(&Json::Num(-150.0)));
        assert_eq!(
            r.get("a"),
            Some(&Json::Arr(vec![Json::Num(1.0), Json::Str("é😀".into())]))
        );
        let e =
            Json::parse(r#"{"method":"Page.loadEventFired","params":{},"sessionId":"S"}"#).unwrap();
        assert_eq!(e.get("id"), None);
        for bad in [
            "",
            "{",
            r#"{"a":}"#,
            r#"{"a":1,}"#,
            "[1 2]",
            r#""\x""#,
            "{} x",
        ] {
            assert_eq!(Json::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn base64_round_trips() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("Zg==").unwrap(), b"f");
        assert_eq!(base64_decode("Zm8=").unwrap(), b"fo");
        assert_eq!(base64_decode("Zm9v").unwrap(), b"foo");
        assert_eq!(base64_decode("Zm9vYmFy").unwrap(), b"foobar");
        assert_eq!(base64_decode("/+8=").unwrap(), [0xff, 0xef]);
        assert_eq!(base64_decode("Zm9v!"), None);
    }

    #[test]
    fn json_strings_are_escaped() {
        assert_eq!(json_string("a\"b\\c\n\u{1}é"), r#""a\"b\\c\n\u0001é""#);
        let back = Json::parse(&json_string("/路径 with \"quotes\"\t")).unwrap();
        assert_eq!(back.as_str(), Some("/路径 with \"quotes\"\t"));
    }

    #[test]
    fn a_chrome_that_ignores_the_pipe_fails_fast() {
        // `true` exits at once: the pipe breaks or closes and the capture reports it, well before
        // the timeout (this is what lets the caller fall back to --screenshot).
        let limits = Limits::none().with_timeout(Duration::from_secs(20));
        let started = Instant::now();
        let err = capture_png(
            Path::new("/usr/bin/true"),
            &[],
            &scratch("true"),
            "about:blank",
            8,
            8,
            &limits,
        )
        .unwrap_err();
        assert!(matches!(err, CdpError::Failed(_)), "{err:?}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(scratch("true"));
    }

    #[test]
    fn a_hung_chrome_times_out() {
        let limits = Limits::none().with_timeout(Duration::from_millis(300));
        let started = Instant::now();
        let args = ["-c".to_string(), "sleep 30".to_string()];
        let err = capture_png(
            Path::new("/bin/sh"),
            &args,
            &scratch("sh"),
            "x",
            8,
            8,
            &limits,
        )
        .unwrap_err();
        assert_eq!(err, CdpError::TimedOut { secs: 0 });
        assert!(started.elapsed() < Duration::from_secs(5));
        let _ = std::fs::remove_dir_all(scratch("sh"));
    }
}
