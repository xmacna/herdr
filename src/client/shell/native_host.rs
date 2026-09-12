//! Opt-in native Windows host inside Herdr. ANSI client/server are unchanged.
use super::{
    native_observer as observe, native_scene_probe as scene, organization, ClientShellConfig,
    ClientShellState,
};
use crate::{
    api::client::{ApiClient, ConnectionTarget},
    config::Config,
    protocol::{ClientShellSnapshot, FrameData},
};
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
pub(super) type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
#[path = "../../platform/windows/native_paint.rs"]
mod paint;
#[path = "../../platform/windows/native_window.rs"]
mod window;
#[derive(Clone)]
pub(super) struct Options {
    pub(super) client: PathBuf,
    pub(super) api: PathBuf,
    pub(super) config: PathBuf,
    pane: String,
    capture: Option<PathBuf>,
    check: bool,
    input_plan: Option<PathBuf>,
    input_events: Option<PathBuf>,
    inspect: Option<PathBuf>,
}
impl Options {
    fn parse(args: &[String]) -> Result<Self> {
        let mut values = std::collections::HashMap::new();
        let mut check = false;
        let mut i = 0;
        while i < args.len() {
            if args[i] == "--check" {
                check = true;
                i += 1;
                continue;
            }
            if ![
                "--client-socket",
                "--api-socket",
                "--config",
                "--pane",
                "--capture",
                "--isolated-input-plan",
                "--input-events",
                "--inspect",
            ]
            .contains(&args[i].as_str())
            {
                return Err(format!("unknown native-demo argument {}", args[i]).into());
            }
            let value = args.get(i + 1).ok_or("missing argument value")?;
            if values.insert(args[i].clone(), value.clone()).is_some() {
                return Err("duplicate native-demo argument".into());
            }
            i += 2;
        }
        let path = |name: &str| -> Result<PathBuf> {
            let p = PathBuf::from(values.get(name).ok_or(format!("{name} required"))?);
            if !p.is_absolute() {
                return Err(format!("{name} must be absolute").into());
            }
            Ok(p)
        };
        let pane = values.get("--pane").ok_or("--pane required")?.clone();
        if pane.is_empty() {
            return Err("empty pane".into());
        }
        Ok(Self {
            client: path("--client-socket")?,
            api: path("--api-socket")?,
            config: path("--config")?,
            pane,
            capture: if values.contains_key("--capture") {
                Some(path("--capture")?)
            } else {
                None
            },
            inspect: if values.contains_key("--inspect") {
                Some(path("--inspect")?)
            } else {
                None
            },
            input_plan: if values.contains_key("--isolated-input-plan") {
                Some(path("--isolated-input-plan")?)
            } else {
                None
            },
            input_events: if values.contains_key("--input-events") {
                Some(path("--input-events")?)
            } else {
                None
            },
            check,
        })
    }
}
struct LiveData {
    snapshot: ClientShellSnapshot,
    organization: crate::config::OrganizationConfig,
    frame: FrameData,
    pane: String,
    terminal: String,
    key: String,
    bytes: usize,
    hash: String,
    collected: u128,
}
fn collect(options: &Options, pane_id: &str) -> Result<LiveData> {
    use sha2::{Digest, Sha256};
    let config: Config = toml::from_str(&std::fs::read_to_string(&options.config)?)?;
    let mut local = ClientShellConfig::from_config(&config);
    local.local_config_path = options.config.clone();
    let mut state = ClientShellState::new(local);
    state.reload_organization_registry();
    if state.endpoint_error.is_some() {
        return Err("registry reload failed".into());
    }
    let api = ApiClient::for_target(ConnectionTarget::SocketPath(options.api.clone()));
    let before = observe::session_snapshot(&api)?;
    let pane = before
        .panes
        .iter()
        .find(|p| p.pane_id == pane_id)
        .ok_or("selected pane no longer exists")?;
    let snapshot = observe::endpoint_snapshot(&options.client)?;
    if let Some(path) = &options.input_plan {
        let plan = super::native_input::Plan::load(path, options)?;
        if snapshot.boot_id != plan.boot_id
            || plan.allowed_terminals.get(pane_id) != Some(&pane.terminal_id)
            || !before.agents.is_empty()
        {
            return Err("isolated observation/input binding invalid".into());
        }
    }
    if !snapshot.panes.iter().any(|p| {
        p.pane_id == pane.pane_id && p.tab_id == pane.tab_id && p.workspace_id == pane.workspace_id
    }) {
        return Err("endpoint/API identity mismatch".into());
    }
    let rows = organization::rows(
        &snapshot,
        &state.config.organization,
        "local",
        &HashSet::new(),
        None,
    );
    let selected = rows
        .iter()
        .find(|r| r.pane_id.as_deref() == Some(pane_id) && !r.invalid);
    if selected.is_none() && !(options.input_plan.is_some() && snapshot.agents.is_empty()) {
        return Err("selected identity absent from organization".into());
    }
    let rect = before
        .layouts
        .iter()
        .flat_map(|l| &l.panes)
        .find(|p| p.pane_id == pane_id)
        .ok_or("missing existing geometry")?
        .rect;
    let ansi =
        observe::terminal_frame(&options.client, &pane.terminal_id, rect.width, rect.height)?;
    let after = observe::session_snapshot(&api)?;
    if !after
        .panes
        .iter()
        .any(|p| p.pane_id == pane_id && p.terminal_id == pane.terminal_id)
    {
        return Err("terminal identity changed during observation".into());
    }
    if before.focused_pane_id != after.focused_pane_id || before.layouts != after.layouts {
        return Err("concurrent focus/layout change: retry a coherent observation".into());
    }
    let frame = crate::pane::render_observed_ansi(&ansi)?;
    Ok(LiveData {
        snapshot,
        organization: state.config.organization,
        frame,
        pane: pane_id.into(),
        terminal: pane.terminal_id.clone(),
        key: selected.map(|s| s.key.clone()).unwrap_or_default(),
        bytes: ansi.bytes.len(),
        hash: format!("{:x}", Sha256::digest(&ansi.bytes)),
        collected: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis(),
    })
}
struct InputCommand {
    pane: String,
    terminal: String,
    boot: String,
    event: crate::protocol::ClientPaneInputEvent,
}
struct View {
    input: Option<std::sync::mpsc::SyncSender<InputCommand>>,
    input_errors: Arc<Mutex<Option<String>>>,
    data: Option<LiveData>,
    scene: Option<scene::Scene>,
    selected: String,
    reserve: bool,
    scroll: f32,
    terminal_only: bool,
    error: Option<String>,
    request: Arc<Mutex<String>>,
    updates: std::sync::mpsc::Receiver<std::result::Result<LiveData, (String, String)>>,
}
impl View {
    fn rebuild(&mut self) {
        self.scene = self
            .data
            .as_ref()
            .and_then(|d| match project_data(d, self.reserve) {
                Ok(s) => Some(s),
                Err(e) => {
                    self.error = Some(e);
                    None
                }
            });
    }
    fn poll(&mut self) -> bool {
        let mut changed = false;
        if let Some(e) = self.input_errors.lock().unwrap().take() {
            self.error = Some(e);
            self.input = None;
            changed = true;
        }
        while let Ok(update) = self.updates.try_recv() {
            match update {
                Ok(data) if data.pane == self.selected => {
                    self.data = Some(data);
                    self.error = None;
                    self.rebuild();
                    changed = true;
                }
                Err((pane, e)) if pane == self.selected => {
                    self.error = Some(e);
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }
    fn input_event(&mut self, event: crate::protocol::ClientPaneInputEvent) {
        if self.error.is_some() {
            return;
        }
        if let (Some(tx), Some(data)) = (&self.input, &self.data) {
            if tx
                .try_send(InputCommand {
                    pane: data.pane.clone(),
                    terminal: data.terminal.clone(),
                    boot: data.snapshot.boot_id.clone(),
                    event,
                })
                .is_err()
            {
                self.error = Some("Fila de entrada indisponível; evento não enviado".into());
            }
        }
    }
    fn click(&mut self, x: f32, y: f32) {
        if x > 1010.0 && y < 64.0 {
            self.terminal_only = !self.terminal_only;
            return;
        }
        if self.terminal_only || x > 440.0 || y < 70.0 {
            return;
        }
        if let Some(hit) = self.scene.as_ref().and_then(|s| s.hit(x, y + self.scroll)) {
            match hit {
                scene::Hit::ToggleReserve => {
                    self.reserve = !self.reserve;
                    self.rebuild();
                }
                scene::Hit::Select { pane_id, .. } => {
                    self.selected = pane_id.clone();
                    *self.request.lock().unwrap() = pane_id;
                    self.data = None;
                    self.scene = None;
                    self.error = None;
                }
            }
        }
    }
}
fn project_data(d: &LiveData, reserve: bool) -> std::result::Result<scene::Scene, String> {
    if !d.key.is_empty() {
        return scene::project_observed(
            &d.snapshot,
            &d.organization,
            d.frame.clone(),
            &d.key,
            reserve,
        );
    }
    // Generic isolated shell has no agent registration; never invent an agent for it.
    Ok(scene::Scene {
        cards: vec![],
        reserve_keys: vec![],
        reserve_open: false,
        toggle: scene::Bounds(0., 0., 0., 0.),
        #[cfg(test)]
        surface: None,
        frame: d.frame.clone(),
        selected_pane: d.pane.clone(),
        selected_label: "Terminal isolado de validação".into(),
        height: 900,
    })
}
pub(crate) fn run(args: &[String]) -> Result<()> {
    if args == ["--help"] {
        println!("herdr native-demo --client-socket ABS --api-socket ABS --config ABS --pane ID [--capture ABS.png | --check]\nExperimental observation host. Click selects locally; wheel scrolls organization; F6 expands terminal. Default: no input, focus, resize, server startup or config writes. --isolated-input-plan ABS.json enables semantic input only for verified isolated shells. --inspect ABS.json exports passive boot/identity metadata. --input-events ABS.json executes a finite isolated input proof without a window. --capture uses real observation and native WIC, without a window.");
        return Ok(());
    }
    let options = Options::parse(args)?;
    if options.input_events.is_some() && options.input_plan.is_none() {
        return Err("--input-events requires --isolated-input-plan".into());
    }
    if options.capture.is_some() && options.input_events.is_some() {
        return Err("capture and input execution are separate modes".into());
    }
    if options.check {
        println!("native-demo arguments valid; no connection or window created");
        return Ok(());
    }
    if let Some(output) = &options.inspect {
        if options.capture.is_some()
            || options.input_events.is_some()
            || options.input_plan.is_some()
        {
            return Err("inspect is read-only and exclusive".into());
        }
        let api = ApiClient::for_target(ConnectionTarget::SocketPath(options.api.clone()));
        let before = observe::session_snapshot(&api)?;
        let snapshot = observe::endpoint_snapshot(&options.client)?;
        let terminals: std::collections::BTreeMap<_, _> = before
            .panes
            .iter()
            .map(|p| (p.pane_id.clone(), p.terminal_id.clone()))
            .collect();
        std::fs::write(
            output,
            serde_json::to_vec_pretty(
                &serde_json::json!({"boot_id":snapshot.boot_id,"terminals":terminals,"agents":before.agents.len(),"readonly":true,"window_created":false}),
            )?,
        )?;
        return Ok(());
    }
    if let Some(plan_path) = &options.input_plan {
        let plan = super::native_input::Plan::load(plan_path, &options)?;
        if let Some(events_path) = &options.input_events {
            if !std::fs::canonicalize(events_path)?.starts_with(std::fs::canonicalize(&plan.root)?)
            {
                return Err("input events outside isolated root".into());
            }
            let bytes = std::fs::read(events_path)?;
            if bytes.len() > 131072 {
                return Err("input fixture too large".into());
            }
            let events: Vec<crate::protocol::ClientPaneInputEvent> =
                serde_json::from_slice(&bytes)?;
            let terminal = plan
                .allowed_terminals
                .get(&options.pane)
                .ok_or("pane not in isolated plan")?;
            plan.send(&options, &options.pane, terminal, &plan.boot_id, events)?;
            println!("isolated semantic input written; verify shell output separately; no window created");
            return Ok(());
        }
    }
    if let Some(output) = &options.capture {
        let data = collect(&options, &options.pane)?;
        let scene = project_data(&data, false)?;
        paint::capture(&scene, output)?;
        let raster_metrics: serde_json::Value =
            serde_json::from_slice(&std::fs::read(output.with_extension("json"))?)?;
        std::fs::write(
            output.with_extension("json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"origin":"live TerminalFrame -> existing Ghostty -> existing pane renderer -> Direct2D", "runtime_connected":true,"raster_operation_metrics":raster_metrics, "pane":data.pane,"terminal":data.terminal,"key":data.key,"agents":data.snapshot.agents.len(),"observed_unix_ms":data.collected,"ansi_bytes":data.bytes,"ansi_sha256":data.hash,"width":data.frame.width,"height":data.frame.height,"window_created":false,"input_sent":false,"semantic_surface_from_server":false,"m0_complete":false,"p1_complete":false}),
            )?,
        )?;
        return Ok(());
    }
    let request = Arc::new(Mutex::new(options.pane.clone()));
    let reader_request = request.clone();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let input_errors = Arc::new(Mutex::new(None));
    let input = options.input_plan.as_ref().map(|plan_path| {
        let (itx, irx) = std::sync::mpsc::sync_channel::<InputCommand>(32);
        let opts = options.clone();
        let path = plan_path.clone();
        let errors = input_errors.clone();
        let selection = reader_request.clone();
        std::thread::spawn(move || {
            while let Ok(command) = irx.recv() {
                if *selection.lock().unwrap() != command.pane {
                    continue;
                }
                let result = super::native_input::Plan::load(&path, &opts).and_then(|plan| {
                    plan.send(
                        &opts,
                        &command.pane,
                        &command.terminal,
                        &command.boot,
                        vec![command.event],
                    )
                });
                if let Err(e) = result {
                    *errors.lock().unwrap() = Some(format!("Entrada revogada: {e}"));
                    break;
                }
            }
        });
        itx
    });
    let mut view = View {
        input,
        input_errors,
        data: None,
        scene: None,
        selected: options.pane.clone(),
        reserve: false,
        scroll: 0.,
        terminal_only: false,
        error: None,
        request,
        updates: rx,
    };
    std::thread::spawn(move || loop {
        let selected = reader_request.lock().unwrap().clone();
        let update = collect(&options, &selected).map_err(|e| (selected, e.to_string()));
        if tx.send(update).is_err() {
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    });
    window::run(&mut view)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn p1_native_host_observed_unicode_and_styles_use_ghostty() {
        let ansi = crate::protocol::TerminalFrame {
            seq: 1,
            width: 100,
            height: 34,
            full: true,
            bytes: "\x1b[2J\x1b[HOlá 界 ┌─┐\r\n\x1b[1;38;2;210;120;40mATENÇÃO\x1b[0m"
                .as_bytes()
                .to_vec(),
        };
        let cells = crate::pane::render_observed_ansi(&ansi).unwrap();
        assert_eq!((cells.width, cells.height), (100, 34));
        let text: String = cells.cells.iter().map(|c| c.symbol.as_str()).collect();
        assert!(text.contains("Olá 界"));
        assert!(text.contains("ATENÇÃO"));
        assert!(cells
            .cells
            .iter()
            .any(|c| c.modifier & ratatui::style::Modifier::BOLD.bits() != 0));
        let mut delta = ansi;
        delta.full = false;
        assert!(crate::pane::render_observed_ansi(&delta).is_err());
    }
    #[test]
    fn p1_native_host_cli_observation_default_no_implicit_input() {
        let args: Vec<String> = [
            "--client-socket",
            "F:/fixture/client.sock",
            "--api-socket",
            "F:/fixture/api.sock",
            "--config",
            "F:/fixture/config.toml",
            "--pane",
            "w1:p1",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let options = Options::parse(&args).unwrap();
        assert!(options.input_plan.is_none());
        assert!(options.input_events.is_none());
        let mut unknown = args.clone();
        unknown.extend(["--takeover".into(), "true".into()]);
        assert!(Options::parse(&unknown).is_err());
        let plan = super::super::native_input::Plan {
            root: PathBuf::from("F:/"),
            client_socket: options.client.clone(),
            api_socket: options.api.clone(),
            config: options.config.clone(),
            server_pid: 0,
            boot_id: String::new(),
            allowed_terminals: Default::default(),
        };
        assert!(plan
            .send(
                &options,
                "w1:p1",
                "terminal",
                "boot",
                vec![crate::protocol::ClientPaneInputEvent::Paste(
                    "must never send".into()
                )]
            )
            .is_err());
    }
}
