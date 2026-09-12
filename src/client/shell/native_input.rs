//! Explicit isolated semantic input. Not reachable by the default observation host.
use super::native_host::{Options, Result};
use super::native_observer as observe;
use crate::{
    api::client::{ApiClient, ConnectionTarget},
    protocol::{self, ClientMessage, ClientPaneInputEvent, ClientSurfaceSize},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub(super) struct ActivePermit {
    _private: (),
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Plan {
    pub root: PathBuf,
    pub client_socket: PathBuf,
    pub api_socket: PathBuf,
    pub config: PathBuf,
    pub server_pid: u32,
    pub boot_id: String,
    pub allowed_terminals: BTreeMap<String, String>,
}
fn canonical(p: &Path) -> Result<PathBuf> {
    if !p.is_absolute() {
        return Err("isolation path must be absolute".into());
    }
    Ok(std::fs::canonicalize(p)?)
}
impl Plan {
    pub(super) fn load(path: &Path, options: &Options) -> Result<Self> {
        let plan: Self = serde_json::from_str(&std::fs::read_to_string(path)?)?;
        plan.validate(options)?;
        if !canonical(path)?.starts_with(canonical(&plan.root)?) {
            return Err("plan outside isolated root".into());
        }
        Ok(plan)
    }
    fn validate(&self, o: &Options) -> Result<()> {
        let root = canonical(&self.root)?;
        if root.parent().is_none() || root.components().count() < 3 {
            return Err("isolation root too broad".into());
        }
        for (actual, planned) in [
            (&o.client, &self.client_socket),
            (&o.api, &self.api_socket),
            (&o.config, &self.config),
        ] {
            let actual = canonical(actual)?;
            if actual != canonical(planned)? || !actual.starts_with(&root) {
                return Err("socket/config outside exact isolated root".into());
            }
        }
        if self.server_pid == 0 || self.boot_id.is_empty() || self.allowed_terminals.is_empty() {
            return Err("incomplete isolated identity plan".into());
        }
        for p in [&self.client_socket, &self.api_socket] {
            let marker = std::fs::read_to_string(p)?;
            if marker.split(':').next().and_then(|v| v.parse::<u32>().ok()) != Some(self.server_pid)
            {
                return Err("isolated server PID marker mismatch".into());
            }
        }
        // An alternate path/alias must not authorize control of the existing production server.
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let production = PathBuf::from(appdata).join("herdr");
            if let Ok(prod) = std::fs::canonicalize(&production) {
                if root.starts_with(&prod) || prod.starts_with(&root) {
                    return Err("production state cannot be isolation root".into());
                }
            }
            for name in ["herdr-client.sock", "herdr.sock"] {
                if let Ok(marker) = std::fs::read_to_string(production.join(name)) {
                    if marker.split(':').next().and_then(|s| s.parse::<u32>().ok())
                        == Some(self.server_pid)
                    {
                        return Err("production server input prohibited".into());
                    }
                }
            }
        }
        // Verify a live executable under the isolated root, not only a stale marker.
        unsafe {
            use windows_sys::Win32::{Foundation::*, System::Threading::*};
            let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, self.server_pid);
            if handle.is_null() {
                return Err("isolated server process unavailable".into());
            }
            let mut path = vec![0u16; 32768];
            let mut length = path.len() as u32;
            let mut exit_code = 0;
            let queried = QueryFullProcessImageNameW(handle, 0, path.as_mut_ptr(), &mut length);
            let running = GetExitCodeProcess(handle, &mut exit_code);
            CloseHandle(handle);
            if queried == 0 || running == 0 || exit_code != 259 {
                return Err("isolated PID is not a live process".into());
            }
            let image = canonical(&PathBuf::from(String::from_utf16(
                &path[..length as usize],
            )?))?;
            if !image.starts_with(&root)
                || !image
                    .file_name()
                    .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("herdr.exe"))
            {
                return Err("isolated server executable outside root".into());
            }
        }
        Ok(())
    }
    pub(super) fn send(
        &self,
        o: &Options,
        pane: &str,
        terminal: &str,
        boot: &str,
        events: Vec<ClientPaneInputEvent>,
    ) -> Result<()> {
        self.validate(o)?;
        if boot != self.boot_id
            || self.allowed_terminals.get(pane).map(String::as_str) != Some(terminal)
        {
            return Err("input target not in isolated identity plan".into());
        }
        if events.iter().any(|e| {
            !matches!(
                e,
                ClientPaneInputEvent::Key { .. }
                    | ClientPaneInputEvent::TextCommit(_)
                    | ClientPaneInputEvent::Paste(_)
            )
        }) {
            return Err("only keyboard/text/paste authorized in input demo".into());
        }
        if events.is_empty()||events.len()>64||events.iter().any(|e|matches!(e,ClientPaneInputEvent::TextCommit(s)|ClientPaneInputEvent::Paste(s) if s.len()>65536)){return Err("input event bound exceeded".into());}
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(o.api.clone()));
        let snapshot = observe::session_snapshot(&api)?;
        if !snapshot.agents.is_empty() {
            return Err("input demo permits isolated shells only, no agents".into());
        }
        if !snapshot
            .panes
            .iter()
            .any(|p| p.pane_id == pane && p.terminal_id == terminal)
        {
            return Err("input pane/terminal no longer matches".into());
        }
        let guard = observe::endpoint_snapshot(&o.client)?;
        if guard.boot_id != self.boot_id {
            return Err("input server boot changed".into());
        }
        let size = snapshot
            .layouts
            .iter()
            .flat_map(|l| &l.panes)
            .find(|p| p.pane_id == pane)
            .ok_or("isolated pane geometry missing")?
            .rect;
        let (mut stream, active) = observe::isolated_endpoint(
            &o.client,
            ClientSurfaceSize {
                cols: size.width,
                rows: size.height,
            },
            &ActivePermit { _private: () },
        )?;
        if active.boot_id != self.boot_id || !active.panes.iter().any(|p| p.pane_id == pane) {
            return Err("active endpoint identity changed".into());
        }
        self.validate(o)?;
        let final_identity = observe::session_snapshot(&api)?;
        if !final_identity.agents.is_empty()
            || !final_identity
                .panes
                .iter()
                .any(|p| p.pane_id == pane && p.terminal_id == terminal)
        {
            return Err("input identity changed after handshake".into());
        }
        // Only this explicit isolated path activates a client and delivers semantic keys/Paste.
        protocol::write_message(
            &mut stream,
            &ClientMessage::ClientShellPaneInput {
                pane_id: pane.into(),
                events,
            },
        )?;
        Ok(())
    }
}
