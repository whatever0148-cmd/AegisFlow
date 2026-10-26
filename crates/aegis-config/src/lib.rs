//! Credential and endpoint settings for AegisFlow.
//!
//! Resolution order per key: the process environment, then the repository's `.env` file.
//! [`Settings`] never mutates the environment, so it is safe to reload from any thread; binaries
//! that hand configuration to `from_env()` constructors call [`export_to_process_env`] once at
//! startup, before spawning threads.
//!
//! `.env` is written with mode 0600 through a uniquely named temporary file (`O_CREAT|O_EXCL`) and
//! an atomic rename. Writers are serialised: a process-wide mutex orders threads of one process and
//! an `flock(2)` on the sidecar `<.env>.lock` orders processes (daemon and CLI). Secret values are
//! only ever returned by [`Settings::get`]; [`Settings::describe`] reports presence and source.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::raw::c_int;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// A setting the console and CLI loader know how to configure.
#[derive(Debug, Clone, Copy)]
pub struct KeySpec {
    pub name: &'static str,
    /// Write-only: the value is never echoed back by any AegisFlow surface.
    pub secret: bool,
    pub hint: &'static str,
    pub signup_url: Option<&'static str>,
}

pub const KEYS: &[KeySpec] = &[
    KeySpec {
        name: "NEBIUS_API_KEY",
        secret: true,
        hint: "API key for the selected Nemotron provider (NEBIUS_BASE_URL: Nebius Token Factory, NVIDIA NIM `nvapi-…` \
               keys from build.nvidia.com, or OpenRouter); recovery tiers 1-2. It is only ever sent to that provider \
               (or a loopback test endpoint)",
        signup_url: Some(NEBIUS_SIGNUP_URL),
    },
    KeySpec {
        name: "NEBIUS_BASE_URL",
        secret: false,
        hint: "Nemotron provider (OpenAI-compatible base URL): https://api.tokenfactory.nebius.com/v1 (Nebius Token Factory, \
               the default), https://integrate.api.nvidia.com/v1 (NVIDIA NIM) or https://openrouter.ai/api/v1 (OpenRouter). \
               Any other value is refused; http(s) loopback (127.0.0.1, localhost, [::1]) is accepted from the CLI or the \
               environment for local tests only. NEBIUS_API_KEY is sent to it",
        signup_url: None,
    },
    KeySpec {
        name: "NEBIUS_TIER1_MODEL",
        secret: false,
        hint: "Tier-1 (Nemotron Super) model id; overrides the provider default, required where the provider has none",
        signup_url: None,
    },
    KeySpec {
        name: "NEBIUS_TIER2_MODEL",
        secret: false,
        hint: "Tier-2 (Nemotron Ultra) model id; overrides the provider default, required where the provider has none",
        signup_url: None,
    },
    KeySpec {
        name: "TAVILY_API_KEY",
        secret: true,
        hint: "Tavily search API key (tier-2 documentation retrieval)",
        signup_url: Some("https://app.tavily.com/"),
    },
    KeySpec {
        name: "PAYPAL_CLIENT_ID",
        secret: true,
        hint: "PayPal Sandbox REST app client id",
        signup_url: Some("https://developer.paypal.com/dashboard/applications/sandbox"),
    },
    KeySpec {
        name: "PAYPAL_CLIENT_SECRET",
        secret: true,
        hint: "PayPal Sandbox REST app secret",
        signup_url: Some("https://developer.paypal.com/dashboard/applications/sandbox"),
    },
    KeySpec {
        name: "AEGIS_MATTERMOST_WEBHOOK_URL",
        secret: true,
        hint: "Mattermost incoming-webhook URL for approval cards",
        signup_url: None,
    },
    KeySpec {
        name: "AEGIS_SLACK_WEBHOOK_URL",
        secret: true,
        hint: "Slack incoming-webhook URL for approval cards",
        signup_url: None,
    },
    KeySpec {
        name: "AEGIS_APPROVAL_CALLBACK_URL",
        secret: false,
        hint: "Mattermost interactive-action callback URL",
        signup_url: None,
    },
];

pub fn spec(name: &str) -> Option<&'static KeySpec> {
    KEYS.iter().find(|k| k.name == name)
}

const NEBIUS_SIGNUP_URL: &str = "https://tokenfactory.nebius.com/";
/// Nebius Token Factory's Nemotron Super / Ultra ids (tiers 1 and 2 since phase 1).
pub const NEBIUS_TIER1_MODEL_ID: &str = "nvidia/Llama-3_3-Nemotron-Super-49B-v1";
pub const NEBIUS_TIER2_MODEL_ID: &str = "nvidia/Llama-3_1-Nemotron-Ultra-253B-v1";

/// A trusted OpenAI-compatible endpoint serving Nemotron models (`NEBIUS_BASE_URL`).
///
/// Default tier models are filled in only where the id was confirmed against the provider's own
/// model list; `None` means `NEBIUS_TIER1_MODEL` / `NEBIUS_TIER2_MODEL` must be set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NemotronProvider {
    /// Stable identifier for the console and scripts (`nebius`, `nvidia-nim`, `openrouter`).
    pub id: &'static str,
    pub label: &'static str,
    /// The only accepted base URL for this provider (compared after removing one trailing `/`).
    pub base_url: &'static str,
    pub signup_url: &'static str,
    pub key_hint: &'static str,
    pub tier1_model: Option<&'static str>,
    pub tier2_model: Option<&'static str>,
}

/// The allowlist of Nemotron providers an API key may be sent to; the first entry is the default.
///
/// * Nebius Token Factory: ids as used since phase 1 (`nvidia/Llama-3_3-…`, `nvidia/Llama-3_1-…`).
/// * NVIDIA NIM: `GET https://integrate.api.nvidia.com/v1/models` (public) lists
///   `nvidia/llama-3.1-nemotron-ultra-253b-v1` but not `nvidia/llama-3.3-nemotron-super-49b-v1`,
///   so only tier 2 has a default.
/// * OpenRouter: `GET https://openrouter.ai/api/v1/models` (public) lists neither Llama-Nemotron
///   Super 49B nor Ultra 253B, so both tier models must be configured.
pub const NEMOTRON_PROVIDERS: &[NemotronProvider] = &[
    NemotronProvider {
        id: "nebius",
        label: "Nebius Token Factory",
        base_url: "https://api.tokenfactory.nebius.com/v1",
        signup_url: NEBIUS_SIGNUP_URL,
        key_hint: "Nebius Token Factory API key",
        tier1_model: Some(NEBIUS_TIER1_MODEL_ID),
        tier2_model: Some(NEBIUS_TIER2_MODEL_ID),
    },
    NemotronProvider {
        id: "nvidia-nim",
        label: "NVIDIA NIM",
        base_url: "https://integrate.api.nvidia.com/v1",
        signup_url: "https://build.nvidia.com",
        key_hint: "build.nvidia.com API key (nvapi-…)",
        tier1_model: None,
        tier2_model: Some("nvidia/llama-3.1-nemotron-ultra-253b-v1"),
    },
    NemotronProvider {
        id: "openrouter",
        label: "OpenRouter",
        base_url: "https://openrouter.ai/api/v1",
        signup_url: "https://openrouter.ai/keys",
        key_hint: "OpenRouter API key",
        tier1_model: None,
        tier2_model: None,
    },
];

/// `NEBIUS_BASE_URL` when it is unset: Nebius Token Factory.
pub const DEFAULT_NEMOTRON_BASE_URL: &str = NEMOTRON_PROVIDERS[0].base_url;

/// The allowlisted provider whose base URL is exactly `url` (surrounding whitespace and one
/// trailing `/` ignored; scheme, host, case and path must match).
pub fn nemotron_provider(url: &str) -> Option<&'static NemotronProvider> {
    let url = url.trim();
    let url = url.strip_suffix('/').unwrap_or(url);
    NEMOTRON_PROVIDERS.iter().find(|p| p.base_url == url)
}

pub fn nemotron_provider_by_id(id: &str) -> Option<&'static NemotronProvider> {
    NEMOTRON_PROVIDERS.iter().find(|p| p.id == id)
}

/// Where a base URL that passed [`validate_base_url`] points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endpoint {
    Provider(&'static NemotronProvider),
    /// `http(s)://127.0.0.1|localhost|[::1][:port]/…`, for local test servers.
    Loopback,
}

/// Default `(tier 1, tier 2)` model ids for `base_url`: the provider's confirmed defaults, the
/// Nebius ids for a loopback test endpoint, and none for a refused URL.
pub fn default_tier_models(base_url: &str) -> (Option<&'static str>, Option<&'static str>) {
    match validate_base_url(base_url) {
        Ok(Endpoint::Provider(p)) => (p.tier1_model, p.tier2_model),
        Ok(Endpoint::Loopback) => (NEMOTRON_PROVIDERS[0].tier1_model, NEMOTRON_PROVIDERS[0].tier2_model),
        Err(_) => (None, None),
    }
}

/// `Label (url), …` for messages.
pub fn allowed_providers_text() -> String {
    NEMOTRON_PROVIDERS.iter().map(|p| format!("{} ({})", p.label, p.base_url)).collect::<Vec<_>>().join(", ")
}

/// Where a resolved value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Env,
    Dotenv,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Env => "env",
            Source::Dotenv => "dotenv",
        }
    }
}

/// Presence report for one known key; carries no value for secrets.
#[derive(Debug, Clone)]
pub struct KeyStatus {
    pub spec: &'static KeySpec,
    pub source: Option<Source>,
    /// The value itself, only for non-secret keys.
    pub public_value: Option<String>,
}

/// Snapshot of the environment merged with `.env`.
#[derive(Debug, Clone)]
pub struct Settings {
    path: PathBuf,
    values: BTreeMap<String, (String, Source)>,
}

impl Settings {
    /// Reads `path` (a missing file is an empty file) and overlays the process environment.
    /// Environment entries whose name or value is not UTF-8 are ignored.
    pub fn load(path: &Path) -> io::Result<Self> {
        let mut settings = Self::load_isolated(path)?;
        for (key, value) in std::env::vars_os() {
            let (Some(key), Some(value)) = (key.to_str(), value.to_str()) else { continue };
            if !value.trim().is_empty() && (spec(key).is_some() || settings.values.contains_key(key)) {
                settings.values.insert(key.to_string(), (value.to_string(), Source::Env));
            }
        }
        Ok(settings)
    }

    /// Reads `path` only, without the process environment (tests and tools that must not pick up
    /// credentials exported in the caller's shell).
    pub fn load_isolated(path: &Path) -> io::Result<Self> {
        let mut values = BTreeMap::new();
        match fs::read_to_string(path) {
            Ok(text) => {
                for (key, value) in parse_dotenv(&text) {
                    if !value.trim().is_empty() {
                        values.insert(key, (value, Source::Dotenv));
                    }
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(Self { path: path.to_path_buf(), values })
    }

    pub fn load_default() -> io::Result<Self> {
        Self::load(&default_dotenv_path())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(|(v, _)| v.as_str())
    }

    pub fn source(&self, name: &str) -> Option<Source> {
        self.values.get(name).map(|(_, s)| *s)
    }

    pub fn is_set(&self, name: &str) -> bool {
        self.values.contains_key(name)
    }

    /// One entry per [`KEYS`] item, in declaration order.
    pub fn describe(&self) -> Vec<KeyStatus> {
        KEYS.iter()
            .map(|spec| KeyStatus {
                spec,
                source: self.source(spec.name),
                public_value: if spec.secret { None } else { self.get(spec.name).map(str::to_string) },
            })
            .collect()
    }

    /// Every `.env` value whose key is not already in the environment, for `from_env()` callers.
    pub fn dotenv_only(&self) -> impl Iterator<Item = (&str, &str)> {
        self.values.iter().filter(|(_, (_, s))| *s == Source::Dotenv).map(|(k, (v, _))| (k.as_str(), v.as_str()))
    }
}

/// Repository root: `AEGIS_ROOT`, else the workspace this crate was built in.
pub fn repo_root() -> PathBuf {
    match std::env::var_os("AEGIS_ROOT") {
        Some(root) if !root.is_empty() => PathBuf::from(root),
        _ => Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").components().collect(),
    }
}

/// `AEGIS_DOTENV`, else `<repo>/.env`.
pub fn default_dotenv_path() -> PathBuf {
    match std::env::var_os("AEGIS_DOTENV") {
        Some(path) if !path.is_empty() => PathBuf::from(path),
        _ => repo_root().join(".env"),
    }
}

/// Copies `.env`-only values into the process environment so `from_env()` constructors see them.
///
/// Mutating the environment races with concurrent `getenv` calls in any thread, so call this
/// once at the top of `main`, before spawning threads or initialising CoreML. Returns the names
/// that were exported.
pub fn export_to_process_env(settings: &Settings) -> Vec<String> {
    let mut exported = Vec::new();
    for (key, value) in settings.dotenv_only() {
        if std::env::var_os(key).is_none() {
            std::env::set_var(key, value);
            exported.push(key.to_string());
        }
    }
    exported
}

/// Parses `KEY=value` lines: `#` comments, optional `export `, single or double quotes
/// (double quotes understand `\"`, `\\` and `\n`), and ` #` comments after unquoted values.
pub fn parse_dotenv(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((key, raw)) = line.split_once('=') else { continue };
        let key = key.trim();
        if !valid_key(key) {
            continue;
        }
        out.push((key.to_string(), unquote(raw.trim())));
    }
    out
}

fn unquote(raw: &str) -> String {
    if let Some(body) = raw.strip_prefix('"').and_then(|r| r.rfind('"').map(|end| &r[..end])) {
        let mut value = String::with_capacity(body.len());
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                value.push(c);
                continue;
            }
            match chars.next() {
                Some('n') => value.push('\n'),
                Some(other) => value.push(other),
                None => value.push('\\'),
            }
        }
        value
    } else if let Some(body) = raw.strip_prefix('\'').and_then(|r| r.rfind('\'').map(|end| &r[..end])) {
        body.to_string()
    } else {
        raw.split(" #").next().unwrap_or("").trim_end().to_string()
    }
}

fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with(|c: char| c.is_ascii_digit())
        && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn quote(value: &str) -> String {
    let plain = value.chars().all(|c| c.is_ascii_alphanumeric() || "-_./:@+=,".contains(c));
    if plain && !value.is_empty() {
        value.to_string()
    } else {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

/// Checks the OpenAI-compatible base URL an API key will be sent to. Accepted:
///
/// * exactly one of the [`NEMOTRON_PROVIDERS`] base URLs (see [`nemotron_provider`]), or
/// * `http://` or `https://` to `127.0.0.1`, `localhost` or `[::1]` with an optional numeric port
///   and any path (local test endpoints).
///
/// Everything else is refused, including look-alike hosts, `http://` to a provider host, user info
/// (`user@host`) and other paths on a provider host. The error lists the allowed providers and never
/// repeats the value.
pub fn validate_base_url(url: &str) -> Result<Endpoint, String> {
    if let Some(provider) = nemotron_provider(url) {
        return Ok(Endpoint::Provider(provider));
    }
    if is_loopback_url(url) {
        return Ok(Endpoint::Loopback);
    }
    Err(format!(
        "must be one of the allowed Nemotron providers: {}; http(s) to 127.0.0.1, localhost or [::1] is accepted for local tests",
        allowed_providers_text()
    ))
}

/// `http(s)://` + a loopback host + an optional numeric port, then nothing or `/`, `?`, `#`.
fn is_loopback_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("http://")) else { return false };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    // User info, backslashes (read as `/` by WHATWG parsers) and whitespace never pass.
    if authority.chars().any(|c| c == '@' || c == '\\' || c.is_whitespace() || c.is_control()) {
        return false;
    }
    let (host, port) = if authority.starts_with('[') {
        match authority.find(']') {
            Some(end) => (&authority[..=end], &authority[end + 1..]),
            None => return false,
        }
    } else {
        match authority.find(':') {
            Some(colon) => (&authority[..colon], &authority[colon..]),
            None => (authority, ""),
        }
    };
    let port_ok = port.is_empty()
        || port.strip_prefix(':').is_some_and(|p| !p.is_empty() && p.len() <= 5 && p.bytes().all(|b| b.is_ascii_digit()));
    port_ok && ["127.0.0.1", "localhost", "[::1]"].contains(&host)
}

/// Per-key value checks applied by [`write_dotenv`].
fn validate_value(name: &str, value: &str) -> Result<(), String> {
    match name {
        "NEBIUS_BASE_URL" => validate_base_url(value).map(|_| ()),
        _ if spec(name).is_some_and(|s| !s.secret) && looks_like_credential(value) => Err(
            "this looks like an API key, not a value for this public setting (public settings are shown in the \
             console); paste keys only into secret settings such as NEBIUS_API_KEY"
                .to_string(),
        ),
        _ => Ok(()),
    }
}

/// Whether a value pasted into a public setting (shown in the console and `config status`) is
/// probably a credential: a known key prefix, or a long single token with no `/`, `:` or `.` (model
/// ids and URLs always have one of those).
pub fn looks_like_credential(value: &str) -> bool {
    let v = value.trim();
    const PREFIXES: [&str; 8] = ["sk-", "sk_", "nvapi-", "tvly-", "pk_", "ghp_", "xoxb-", "Bearer "];
    PREFIXES.iter().any(|p| v.starts_with(p))
        || (v.len() >= 32
            && !v.contains(['/', ':', '.'])
            && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'))
}

/// Orders writers inside this process (the `flock` below orders processes).
static WRITERS: Mutex<()> = Mutex::new(());

extern "C" {
    /// flock(2) from libSystem / libc.
    fn flock(fd: c_int, operation: c_int) -> c_int;
}
/// `LOCK_EX` and `LOCK_UN` have these values on macOS (`<sys/fcntl.h>`) and Linux alike.
const LOCK_EX: c_int = 2;
const LOCK_UN: c_int = 8;

/// Exclusive `flock` on `<.env>.lock`, released on drop.
struct DotenvLock {
    file: File,
}

impl DotenvLock {
    fn acquire(dotenv: &Path) -> io::Result<Self> {
        let mut name = dotenv.file_name().unwrap_or(OsStr::new(".env")).to_os_string();
        name.push(".lock");
        let path = dotenv.with_file_name(name);
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} is a symbolic link; refusing to use it as the lock file", path.display())));
        }
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).mode(0o600).open(&path)?;
        // The path may have been swapped for a link between the check and the open: the open file
        // must be the regular file now at that path.
        let (opened, now) = (file.metadata()?, fs::symlink_metadata(&path)?);
        if !now.file_type().is_file() || opened.dev() != now.dev() || opened.ino() != now.ino() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{} changed while it was opened", path.display())));
        }
        loop {
            // SAFETY: `file` owns a valid open descriptor for the duration of the call; flock only
            // takes an advisory lock on it and touches no memory. The return code is checked.
            let rc = unsafe { flock(file.as_raw_fd(), LOCK_EX) };
            if rc == 0 {
                return Ok(Self { file });
            }
            let err = io::Error::last_os_error();
            if err.kind() != io::ErrorKind::Interrupted {
                return Err(err);
            }
        }
    }
}

impl Drop for DotenvLock {
    fn drop(&mut self) {
        // SAFETY: same descriptor as in `acquire`, still open; unlocking cannot touch memory. A
        // failure is harmless because closing the descriptor (right after) releases the lock too.
        let _ = unsafe { flock(self.file.as_raw_fd(), LOCK_UN) };
    }
}

/// Creates a new 0600 file next to `path` with a name no other writer uses (pid, counter, clock),
/// with `O_CREAT|O_EXCL`, which also refuses to follow a symbolic link planted at that name.
fn create_temp(dir: &Path, path: &Path) -> io::Result<(PathBuf, File)> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let base = path.file_name().map(OsStr::as_bytes).unwrap_or(b".env");
    let base = String::from_utf8_lossy(base);
    let mut last = None;
    for _ in 0..16 {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{}.tmp-{}-{n}-{nanos:08x}", base.trim_start_matches('.'), std::process::id()));
        match OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp) {
            Ok(file) => return Ok((tmp, file)),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "no free temporary file name")))
}

/// Applies `updates` (`None` or an empty string removes the key) to the `.env` at `path`,
/// keeping other lines and comments in place. Only names in [`KEYS`] are accepted, values may
/// not contain line breaks, `NEBIUS_BASE_URL` must pass [`validate_base_url`], and the result is
/// written 0600 via a unique temp file + rename while holding the writer locks, so concurrent
/// writers (threads or processes) never lose each other's keys or mix their bytes. A `.env` that
/// is a symbolic link is refused (point `AEGIS_DOTENV` at the target file instead).
pub fn write_dotenv(path: &Path, updates: &[(&str, Option<&str>)]) -> io::Result<()> {
    for (name, value) in updates {
        if spec(name).is_none() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("unknown setting {name}")));
        }
        if value.is_some_and(|v| v.contains(['\n', '\r', '\0'])) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{name}: value contains a line break")));
        }
        if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
            validate_value(name, v).map_err(|why| io::Error::new(io::ErrorKind::InvalidInput, format!("{name}: {why}")))?;
        }
    }
    let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let _threads = WRITERS.lock().unwrap_or_else(PoisonError::into_inner);
    let _processes = DotenvLock::acquire(path)?;
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is a symbolic link; not replacing it (point AEGIS_DOTENV at the target file)", path.display()),
        ));
    }
    let existing = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let wanted = |key: &str| updates.iter().rev().find(|(name, _)| *name == key).map(|(_, v)| v.filter(|v| !v.trim().is_empty()));
    let mut written = Vec::new();
    let mut lines = Vec::new();
    for line in existing.lines() {
        let trimmed = line.trim();
        let key = trimmed
            .strip_prefix("export ")
            .unwrap_or(trimmed)
            .split_once('=')
            .map(|(k, _)| k.trim())
            .filter(|k| !trimmed.starts_with('#') && valid_key(k));
        match key.and_then(|k| wanted(k).map(|v| (k, v))) {
            Some((k, Some(v))) if !written.contains(&k) => {
                lines.push(format!("{k}={}", quote(v)));
                written.push(k);
            }
            Some(_) => {}
            None => lines.push(line.to_string()),
        }
    }
    for (name, _) in updates {
        if let Some(v) = wanted(name).flatten() {
            if !written.contains(name) {
                lines.push(format!("{name}={}", quote(v)));
                written.push(name);
            }
        }
    }
    let mut body = lines.join("\n");
    if !body.is_empty() {
        body.push('\n');
    }
    let (tmp, mut file) = create_temp(dir, path)?;
    let written = file
        .set_permissions(fs::Permissions::from_mode(0o600))
        .and_then(|()| file.write_all(body.as_bytes()))
        .and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&tmp, path));
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aegis-config-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir.join(".env")
    }

    #[test]
    fn api_keys_are_refused_in_public_settings_without_echoing_them() {
        let path = scratch("credential-guard");
        let fake = format!("sk-or-v1-{}", "0".repeat(64));
        let fake = fake.as_str();
        for name in ["NEBIUS_TIER1_MODEL", "NEBIUS_TIER2_MODEL"] {
            let err = write_dotenv(&path, &[(name, Some(fake))]).unwrap_err().to_string();
            assert!(err.contains("looks like an API key"), "{err}");
            assert!(!err.contains(fake), "the refused value must not be echoed");
        }
        assert!(looks_like_credential("nvapi-abcdef"));
        assert!(looks_like_credential("tvly-dev-abc"));
        assert!(looks_like_credential("Zm9vYmFyYmF6cXV4cXV1eHF1dXhxdXV4cXV1eA"));
        // Model ids and URLs pass.
        write_dotenv(&path, &[("NEBIUS_TIER1_MODEL", Some("nvidia/nemotron-3-super-120b-a12b:free"))]).unwrap();
        write_dotenv(&path, &[("NEBIUS_TIER2_MODEL", Some("nvidia/Llama-3_1-Nemotron-Ultra-253B-v1"))]).unwrap();
        write_dotenv(&path, &[("AEGIS_APPROVAL_CALLBACK_URL", Some("https://example.test/aegis/approve"))]).unwrap();
        // Secret settings accept keys, of course.
        write_dotenv(&path, &[("NEBIUS_API_KEY", Some(fake))]).unwrap();
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn parses_quotes_comments_and_export() {
        let parsed = parse_dotenv(
            "# comment\nexport A_KEY=plain # trailing\nB_KEY=\"with \\\"quote\\\" and # hash\"\nC_KEY='single # kept'\nlower=skip\n9BAD=x\n",
        );
        assert_eq!(
            parsed,
            vec![
                ("A_KEY".to_string(), "plain".to_string()),
                ("B_KEY".to_string(), "with \"quote\" and # hash".to_string()),
                ("C_KEY".to_string(), "single # kept".to_string()),
            ]
        );
    }

    #[test]
    fn write_merges_preserves_comments_and_sets_0600() {
        let path = scratch("merge");
        fs::write(&path, "# keep me\nTAVILY_API_KEY=old\nOTHER=1\n").unwrap();
        write_dotenv(&path, &[("TAVILY_API_KEY", Some("tvly-new value")), ("NEBIUS_API_KEY", Some("nb-1"))]).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# keep me\n"));
        assert!(text.contains("TAVILY_API_KEY=\"tvly-new value\"\n"));
        assert!(text.contains("OTHER=1\n"));
        assert!(text.contains("NEBIUS_API_KEY=nb-1\n"));
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);

        write_dotenv(&path, &[("TAVILY_API_KEY", None), ("NEBIUS_API_KEY", Some(""))]).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains("TAVILY_API_KEY") && !text.contains("NEBIUS_API_KEY"));
        assert!(text.contains("OTHER=1"));
    }

    #[test]
    fn write_rejects_unknown_names_and_line_breaks() {
        let path = scratch("reject");
        assert!(write_dotenv(&path, &[("NOT_A_KNOWN_KEY", Some("x"))]).is_err());
        assert!(write_dotenv(&path, &[("TAVILY_API_KEY", Some("a\nB=c"))]).is_err());
        assert!(!path.exists());
    }

    #[test]
    fn round_trips_through_settings_without_exposing_secrets() {
        let path = scratch("settings");
        write_dotenv(&path, &[("AEGIS_APPROVAL_CALLBACK_URL", Some("http://127.0.0.1:9/cb")), ("AEGIS_SLACK_WEBHOOK_URL", Some("https://hooks.example/x"))]).unwrap();
        // Isolated: credentials exported in the shell running the tests cannot change the result.
        let settings = Settings::load_isolated(&path).unwrap();
        let slack = settings.describe().into_iter().find(|s| s.spec.name == "AEGIS_SLACK_WEBHOOK_URL").unwrap();
        assert_eq!(slack.source, Some(Source::Dotenv));
        assert_eq!(settings.get("AEGIS_SLACK_WEBHOOK_URL"), Some("https://hooks.example/x"));
        assert!(slack.public_value.is_none());
        let callback = settings.describe().into_iter().find(|s| s.spec.name == "AEGIS_APPROVAL_CALLBACK_URL").unwrap();
        assert_eq!(callback.public_value.as_deref(), Some("http://127.0.0.1:9/cb"));
    }

    #[test]
    fn missing_file_is_empty() {
        let path = scratch("missing");
        let settings = Settings::load(&path).unwrap();
        assert_eq!(settings.dotenv_only().count(), 0);
    }

    #[test]
    fn isolated_load_ignores_the_environment_and_load_overlays_it() {
        let path = scratch("isolated");
        fs::write(&path, "AEGIS_CONFIG_TEST_ISOLATED=from-dotenv\n").unwrap();
        std::env::set_var("AEGIS_CONFIG_TEST_ISOLATED", "from-env");
        let isolated = Settings::load_isolated(&path).unwrap();
        let merged = Settings::load(&path).unwrap();
        std::env::remove_var("AEGIS_CONFIG_TEST_ISOLATED");
        assert_eq!((isolated.get("AEGIS_CONFIG_TEST_ISOLATED"), isolated.source("AEGIS_CONFIG_TEST_ISOLATED")), (Some("from-dotenv"), Some(Source::Dotenv)));
        assert_eq!((merged.get("AEGIS_CONFIG_TEST_ISOLATED"), merged.source("AEGIS_CONFIG_TEST_ISOLATED")), (Some("from-env"), Some(Source::Env)));
    }

    #[test]
    fn non_utf8_environment_entries_do_not_panic() {
        use std::os::unix::ffi::OsStrExt;
        let path = scratch("non-utf8");
        fs::write(&path, "AEGIS_CONFIG_TEST_JUNK=kept\n").unwrap();
        std::env::set_var("AEGIS_CONFIG_TEST_JUNK", OsStr::from_bytes(b"a\xffb"));
        let loaded = Settings::load(&path);
        std::env::remove_var("AEGIS_CONFIG_TEST_JUNK");
        let settings = loaded.unwrap();
        // The unusable environment value is skipped, so the .env value stays in effect.
        assert_eq!(settings.get("AEGIS_CONFIG_TEST_JUNK"), Some("kept"));
    }

    #[test]
    fn concurrent_writers_keep_every_key_and_never_mix_bytes() {
        let path = scratch("concurrent");
        fs::write(&path, "# header\n").unwrap();
        let names = ["NEBIUS_TIER1_MODEL", "NEBIUS_TIER2_MODEL", "AEGIS_APPROVAL_CALLBACK_URL", "TAVILY_API_KEY"];
        for round in 0..8 {
            // Large values make an interleaved write visible as a truncated or stray line.
            let values: Vec<String> = names.iter().enumerate().map(|(i, _)| format!("vendor/r{round}-{}", char::from(b'a' + i as u8).to_string().repeat(120_000))).collect();
            let barrier = std::sync::Barrier::new(names.len());
            std::thread::scope(|s| {
                for (name, value) in names.iter().zip(&values) {
                    let (path, barrier) = (&path, &barrier);
                    s.spawn(move || {
                        barrier.wait();
                        write_dotenv(path, &[(name, Some(value.as_str()))]).unwrap();
                    });
                }
            });
            let text = fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = text.lines().collect();
            assert_eq!(lines.len(), names.len() + 1, "round {round}: one line per key plus the header");
            assert_eq!(lines[0], "# header");
            let parsed: BTreeMap<String, String> = parse_dotenv(&text).into_iter().collect();
            for (name, value) in names.iter().zip(&values) {
                assert!(parsed.get(*name) == Some(value), "round {round}: {name} lost or damaged");
            }
        }
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap()).unwrap().filter_map(Result::ok).filter(|e| e.file_name().to_string_lossy().contains(".tmp-")).collect();
        assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[test]
    fn symlinked_dotenv_is_refused_and_its_target_left_alone() {
        let path = scratch("symlink");
        let dir = path.parent().unwrap().to_path_buf();
        let target = dir.join("victim");
        fs::write(&target, "important user data\n").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        std::os::unix::fs::symlink(&target, &path).unwrap();
        // A link planted at the old fixed temp name is never followed either.
        let planted = dir.join(format!(".env.tmp-{}", std::process::id()));
        std::os::unix::fs::symlink(&target, &planted).unwrap();
        let err = write_dotenv(&path, &[("TAVILY_API_KEY", Some("tvly-SECRETVALUE123"))]).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
        assert!(!err.to_string().contains("tvly-SECRETVALUE123"));
        assert_eq!(fs::read_to_string(&target).unwrap(), "important user data\n");
        assert_eq!(fs::metadata(&target).unwrap().permissions().mode() & 0o777, 0o644);
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_symlink());

        // With a regular .env the write succeeds and still leaves the planted link untouched.
        fs::remove_file(&path).unwrap();
        write_dotenv(&path, &[("TAVILY_API_KEY", Some("tvly-SECRETVALUE123"))]).unwrap();
        assert!(fs::symlink_metadata(&path).unwrap().file_type().is_file());
        assert_eq!(fs::read_to_string(&target).unwrap(), "important user data\n");
        assert!(fs::symlink_metadata(&planted).unwrap().file_type().is_symlink());
    }

    #[test]
    fn base_url_allowlist_accepts_the_three_providers_and_loopback() {
        let expect = [
            ("https://api.tokenfactory.nebius.com/v1", "nebius"),
            ("https://api.tokenfactory.nebius.com/v1/", "nebius"),
            ("https://integrate.api.nvidia.com/v1", "nvidia-nim"),
            ("https://integrate.api.nvidia.com/v1/", "nvidia-nim"),
            ("https://openrouter.ai/api/v1", "openrouter"),
            (" https://openrouter.ai/api/v1/ ", "openrouter"),
        ];
        for (url, id) in expect {
            match validate_base_url(url) {
                Ok(Endpoint::Provider(p)) => assert_eq!(p.id, id, "{url}"),
                other => panic!("{url}: {other:?}"),
            }
        }
        assert_eq!(NEMOTRON_PROVIDERS.len(), 3);
        assert_eq!(DEFAULT_NEMOTRON_BASE_URL, "https://api.tokenfactory.nebius.com/v1");
        assert_eq!(nemotron_provider_by_id("openrouter").map(|p| p.base_url), Some("https://openrouter.ai/api/v1"));
        for ok in [
            "http://127.0.0.1:9/v1",
            "http://localhost:8080/v1",
            "http://[::1]:8080/v1",
            "https://127.0.0.1:8443/v1",
            "https://localhost/v1",
            "http://127.0.0.1",
        ] {
            assert_eq!(validate_base_url(ok), Ok(Endpoint::Loopback), "{ok}");
        }
    }

    #[test]
    fn base_url_allowlist_rejects_everything_else_without_repeating_it() {
        for bad in [
            // Look-alike hosts, wrong scheme, user info, other paths on a provider host.
            "https://integrate.api.nvidia.com.evil.example/v1",
            "http://integrate.api.nvidia.com/v1",
            "https://x@openrouter.ai/api/v1",
            "https://openrouter.ai@evil.example/api/v1",
            "https://openrouter.ai/api/v2",
            "https://openrouter.ai/v1",
            "https://openrouter.ai/api/v1/chat",
            "https://openrouter.ai/api/v1//",
            "https://openrouter.ai:443/api/v1",
            "https://integrate.api.nvidia.com/v1?x=1",
            "https://integrate.api.nvidia.com/v1#frag",
            "https://INTEGRATE.API.NVIDIA.COM/v1",
            "https://api.nvidia.com/v1",
            "https://evilintegrate.api.nvidia.com/v1",
            "https://api.tokenfactory.nebius.com.evil.example/v1",
            "https://api.tokenfactory.nebius.com/v2",
            "http://api.tokenfactory.nebius.com/v1",
            "https://api.studio.nebius.ai/v1",
            "https://proxy.example:8443/v1",
            // Loopback look-alikes and malformed loopback.
            "http://evil.example/v1",
            "http://127.0.0.1.evil.example/v1",
            "http://localhost.evil.example:80/v1",
            "http://user@127.0.0.1:9/v1",
            "http://evil.example\\@127.0.0.1/v1",
            "http://127.0.0.1:9\\@evil.example/v1",
            "http://127.0.0.1:x/v1",
            "http://127.0.0.1:/v1",
            "http://[::1/v1",
            "ftp://127.0.0.1/v1",
            "api.tokenfactory.nebius.com/v1",
            "",
        ] {
            let err = validate_base_url(bad).unwrap_err();
            for provider in NEMOTRON_PROVIDERS {
                assert!(err.contains(provider.base_url) && err.contains(provider.label), "the message lists the providers: {err}");
            }
            // A refused value is never repeated (a value that is part of an allowed URL is
            // necessarily inside the list of allowed URLs, so those are skipped).
            if !bad.is_empty() && !NEMOTRON_PROVIDERS.iter().any(|p| p.base_url.contains(bad.trim())) {
                assert!(!err.contains(bad), "the message repeats {bad:?}: {err}");
            }
            assert_eq!(default_tier_models(bad), (None, None), "{bad}");
        }
    }

    #[test]
    fn tier_model_defaults_follow_the_provider() {
        assert_eq!(
            default_tier_models("https://api.tokenfactory.nebius.com/v1"),
            (Some("nvidia/Llama-3_3-Nemotron-Super-49B-v1"), Some("nvidia/Llama-3_1-Nemotron-Ultra-253B-v1"))
        );
        assert_eq!(default_tier_models("https://integrate.api.nvidia.com/v1/"), (None, Some("nvidia/llama-3.1-nemotron-ultra-253b-v1")));
        assert_eq!(default_tier_models("https://openrouter.ai/api/v1"), (None, None));
        assert_eq!(default_tier_models("http://127.0.0.1:9/v1"), default_tier_models(DEFAULT_NEMOTRON_BASE_URL));
    }

    #[test]
    fn write_dotenv_accepts_only_allowlisted_or_loopback_base_urls() {
        let path = scratch("base-url");
        for bad in ["http://evil.example/v1", "https://integrate.api.nvidia.com.evil.example/v1", "https://x@openrouter.ai/api/v1"] {
            let err = write_dotenv(&path, &[("NEBIUS_BASE_URL", Some(bad))]).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
            assert!(!err.to_string().contains(bad), "{err}");
            assert!(!path.exists(), "nothing written for a refused value");
        }
        for provider in NEMOTRON_PROVIDERS {
            write_dotenv(&path, &[("NEBIUS_BASE_URL", Some(provider.base_url))]).unwrap();
            let settings = Settings::load_isolated(&path).unwrap();
            assert_eq!(settings.get("NEBIUS_BASE_URL"), Some(provider.base_url));
        }
        write_dotenv(&path, &[("NEBIUS_BASE_URL", Some("http://127.0.0.1:9/v1"))]).unwrap();
        write_dotenv(&path, &[("NEBIUS_BASE_URL", None)]).unwrap();
        assert!(!fs::read_to_string(&path).unwrap().contains("NEBIUS_BASE_URL"));
    }
}
