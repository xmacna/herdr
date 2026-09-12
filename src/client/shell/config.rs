// Modified under the Apache License, Version 2.0; see LICENSE.
use super::*;

#[derive(serde::Deserialize)]
struct OrganizationRegistryFile {
    schema_version: u32,
    agents: Vec<crate::config::OrganizationAgentConfig>,
}

const ORGANIZATION_REGISTRY_CHECK_INTERVAL: std::time::Duration =
    std::time::Duration::from_millis(500);

fn organization_registry_fingerprint(metadata: &std::fs::Metadata) -> (u64, u128) {
    (
        metadata.len(),
        metadata
            .modified()
            .ok()
            .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |duration| duration.as_nanos()),
    )
}

pub(super) fn merged_config_diagnostic(
    local: Option<&str>,
    endpoint: Option<&str>,
) -> Option<String> {
    match (local, endpoint) {
        (Some(local), Some(endpoint)) if local == endpoint => {
            Some(format!("client + endpoint: {local}"))
        }
        (Some(local), Some(endpoint)) => Some(format!("client: {local}\nendpoint: {endpoint}")),
        (Some(local), None) => Some(local.to_owned()),
        (None, Some(endpoint)) => Some(endpoint.to_owned()),
        (None, None) => None,
    }
}

impl ClientShellState {
    pub(crate) fn tick_organization_registry(&mut self, now: std::time::Instant) -> bool {
        if self.organization_registry_last_check.is_some_and(|last| {
            now.saturating_duration_since(last) < ORGANIZATION_REGISTRY_CHECK_INTERVAL
        }) {
            return false;
        }
        self.organization_registry_last_check = Some(now);
        let before = self.organization_registry_fingerprint;
        let before_error = self.endpoint_error.clone();
        self.reload_organization_registry();
        before != self.organization_registry_fingerprint || before_error != self.endpoint_error
    }

    fn set_organization_registry_error(&mut self, error: String) {
        if self.endpoint_error.is_none()
            || self
                .endpoint_error
                .as_deref()
                .is_some_and(|current| current.starts_with("organization registry "))
        {
            self.endpoint_error = Some(error);
        }
    }

    fn clear_organization_registry_error(&mut self) {
        if self
            .endpoint_error
            .as_deref()
            .is_some_and(|current| current.starts_with("organization registry "))
        {
            self.endpoint_error = None;
        }
    }

    pub(super) fn reload_organization_registry(&mut self) {
        let Some(registry_path) = self.config.organization.registry_path.as_deref() else {
            self.organization_registry_fingerprint = None;
            self.organization_registry_observed_fingerprint = None;
            self.clear_organization_registry_error();
            return;
        };
        // The registry belongs to this client's config, not the shell's cwd.
        // Path::join preserves an explicitly absolute registry path.
        let path = self
            .config
            .local_config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(registry_path);
        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) => {
                self.organization_registry_observed_fingerprint = None;
                self.set_organization_registry_error(format!(
                    "organization registry unavailable ({}): {error}",
                    path.display()
                ));
                return;
            }
        };
        let fingerprint = organization_registry_fingerprint(&metadata);
        if self.organization_registry_observed_fingerprint == Some(fingerprint) {
            return;
        }
        self.organization_registry_observed_fingerprint = Some(fingerprint);
        let content = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(error) => {
                self.set_organization_registry_error(format!(
                    "organization registry unreadable ({}): {error}",
                    path.display()
                ));
                return;
            }
        };
        let registry = match serde_json::from_str::<OrganizationRegistryFile>(&content) {
            Ok(registry) if registry.schema_version == 1 => registry,
            Ok(registry) => {
                self.set_organization_registry_error(format!(
                    "organization registry schema {} unsupported ({})",
                    registry.schema_version,
                    path.display()
                ));
                return;
            }
            Err(error) => {
                self.set_organization_registry_error(format!(
                    "organization registry invalid ({}): {error}",
                    path.display()
                ));
                return;
            }
        };
        self.config.organization.agents = registry.agents;
        self.organization_registry_fingerprint = Some(fingerprint);
        self.clear_organization_registry_error();
    }

    pub(super) fn set_local_config_diagnostic(&mut self, diagnostic: Option<String>) {
        self.local_config_diagnostic = diagnostic;
        self.config_diagnostic = merged_config_diagnostic(
            self.local_config_diagnostic.as_deref(),
            self.snapshot
                .as_deref()
                .and_then(|snapshot| snapshot.config_diagnostic.as_deref()),
        );
    }

    pub(super) fn persist_chrome_preferences(&mut self, outcome: &mut ClientShellInput) {
        let Some(path) = self.config.preferences_path.as_deref() else {
            return;
        };
        let mut collapsed_groups = self.collapsed_groups.iter().cloned().collect::<Vec<_>>();
        collapsed_groups.sort();
        let preferences = preferences::ClientChromePreferences {
            sidebar_width: self.sidebar_width_manual.then_some(self.sidebar_width),
            sidebar_section_split: self
                .sidebar_section_split_manual
                .then_some(self.sidebar_section_split),
            sidebar_collapsed: self
                .sidebar_collapsed_manual
                .then_some(self.sidebar_collapsed),
            agent_panel_sort: self
                .agent_panel_sort_manual
                .then_some(self.config.agent_panel_sort),
            collapsed_groups,
            organization_collapsed: {
                let mut values = self
                    .organization_collapsed
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>();
                values.sort();
                values
            },
            organization_locale: self
                .organization_locale_manual
                .then_some(self.config.organization_locale.code().to_owned()),
        };
        if let Err(error) = preferences::store(path, preferences) {
            self.endpoint_error = Some(error);
            outcome.repaint = true;
        }
    }

    pub(crate) fn reload_client_config(&mut self) {
        match crate::config::load_live_config() {
            Ok(loaded) => {
                let agent_panel_sort = self.config.agent_panel_sort;
                let organization_locale = self.config.organization_locale;
                let registry_path = self.config.organization.registry_path.clone();
                let registry_agents = self.config.organization.agents.clone();
                let registry_loaded = self.organization_registry_fingerprint.is_some();
                let diagnostics = self.config.apply_live_config(
                    &loaded.config,
                    &loaded.diagnostics,
                    &loaded.invalid_sections,
                );
                if let Some(appearance) = self.host_appearance {
                    self.config.palette = crate::app::client_palette_for_appearance(
                        &self.config.theme_runtime,
                        appearance,
                    );
                }
                if !self.sidebar_width_manual {
                    self.sidebar_width = self.config.sidebar_width;
                }
                if self.agent_panel_sort_manual {
                    self.config.agent_panel_sort = agent_panel_sort;
                }
                if self.organization_locale_manual {
                    self.config.organization_locale = organization_locale;
                }
                if registry_path != self.config.organization.registry_path {
                    self.organization_registry_fingerprint = None;
                    self.organization_registry_observed_fingerprint = None;
                    self.organization_registry_last_check = None;
                } else if registry_loaded {
                    // The JSON registry owns agents while its path is configured. Preserve the
                    // last valid projection while the TOML shell config is reloaded.
                    self.config.organization.agents = registry_agents;
                }
                self.reload_organization_registry();
                self.set_local_config_diagnostic(self.config.local_config_diagnostic(&diagnostics));
                if let Some(snapshot) = self.snapshot.as_deref() {
                    let profile = snapshot.server_keybindings_toml.clone();
                    let commands = snapshot.commands.clone();
                    if let Err(err) = self
                        .config
                        .apply_snapshot_keybindings(profile.as_deref(), &commands)
                    {
                        self.endpoint_error = Some(err);
                    }
                }
            }
            Err(diagnostics) => {
                self.set_local_config_diagnostic(self.config.local_config_diagnostic(&diagnostics));
            }
        }
        self.reconcile_input_source();
    }
}

impl ClientShellConfig {
    pub(crate) fn from_config(config: &Config) -> Self {
        let theme_runtime = crate::app::client_theme_runtime_from_config(config);
        Self {
            sidebar_width: config.ui.sidebar_width,
            sidebar_min_width: config.ui.sidebar_min_width,
            sidebar_max_width: config.ui.sidebar_max_width,
            sidebar_start_collapsed: config.ui.sidebar_start_collapsed,
            sidebar_collapsed_mode: config.ui.sidebar_collapsed_mode,
            mobile_width_threshold: config.ui.mobile_width_threshold,
            tab_bar_position: config.ui.tab_bar_position,
            hide_tab_bar_when_single_tab: config.ui.hide_tab_bar_when_single_tab,
            spaces: config.ui.sidebar.spaces.clone(),
            agents: config.ui.sidebar.agents.clone(),
            agent_panel_sort: config.ui.agent_panel_sort,
            organization: config.ui.organization.clone(),
            organization_locale: config.ui.organization.locale(),
            status_indicators: config.ui.status_indicators,
            sound_enabled: config.ui.sound.enabled,
            toast_delivery: config.ui.toast.delivery,
            toast_delay_seconds: config.ui.toast.delay_seconds,
            toast_position: config.ui.toast.herdr.position,
            copy_on_select: config.ui.copy_on_select,
            clipboard_toast_enabled: config.ui.toast.clipboard.enabled,
            clipboard_toast_position: config.ui.toast.clipboard.position,
            theme_name: theme_runtime.manual_name.clone(),
            theme_runtime,
            palette: crate::app::client_palette_from_config(config),
            keybinds: config
                .live_keybinds_with_diagnostics()
                .map(|(keybinds, _diagnostics)| keybinds)
                .unwrap_or_else(|_diagnostics| LiveKeybindConfig {
                    prefix: config.prefix_key(),
                    keybinds: config.keybinds(),
                }),
            local_keys: config.keys.clone(),
            keybinding_source: ClientShellKeybindingSource::Local,
            prompt_new_tab_name: config.ui.prompt_new_tab_name,
            prompt_new_workspace_name: config.ui.prompt_new_workspace_name,
            confirm_close: config.ui.confirm_close,
            mouse_capture: config.ui.mouse_capture,
            mouse_scroll_lines: config.ui.mouse_scroll_lines(),
            right_click_passthrough_modifiers: config.ui.right_click_passthrough_modifiers(),
            redraw_on_focus_gained: config.ui.redraw_on_focus_gained,
            switch_ascii_input_source_in_prefix: config
                .experimental
                .switch_ascii_input_source_in_prefix,
            local_config_path: crate::config::config_path(),
            preferences_path: None,
            preferences: preferences::ClientChromePreferences::default(),
            startup_config_diagnostic: None,
            startup_onboarding: false,
        }
    }

    pub(crate) fn with_startup_config_diagnostic(mut self, diagnostic: Option<String>) -> Self {
        self.startup_config_diagnostic = diagnostic;
        self
    }

    pub(crate) fn with_startup_onboarding(mut self, show: bool) -> Self {
        self.startup_onboarding = show;
        self
    }

    pub(crate) fn with_keybinding_source(mut self, source: ClientShellKeybindingSource) -> Self {
        self.keybinding_source = source;
        self.keybinds.keybinds.custom_commands.clear();
        self
    }

    pub(crate) fn uses_endpoint_keybindings(&self) -> bool {
        self.keybinding_source == ClientShellKeybindingSource::Endpoint
    }

    pub(super) fn local_config_diagnostic(&self, diagnostics: &[String]) -> Option<String> {
        if self.uses_endpoint_keybindings() {
            crate::config::config_diagnostic_summary_without_keybindings(diagnostics)
        } else {
            crate::config::config_diagnostic_summary(diagnostics)
        }
    }

    pub(crate) fn with_local_endpoint(self, socket_path: &std::path::Path) -> Self {
        self.with_preferences_path(preferences::path_for_local_endpoint(socket_path))
    }

    pub(super) fn with_preferences_path(mut self, path: std::path::PathBuf) -> Self {
        self.preferences = preferences::load(&path).unwrap_or_default();
        self.preferences_path = Some(path);
        self
    }

    pub(super) fn apply_snapshot_keybindings(
        &mut self,
        profile: Option<&str>,
        commands: &[crate::protocol::ClientShellCommand],
    ) -> Result<(), String> {
        let mut keybinds = match self.keybinding_source {
            ClientShellKeybindingSource::Endpoint => crate::config::keybindings_from_profile_toml(
                profile.ok_or("endpoint did not publish its keybindings")?,
            )?,
            ClientShellKeybindingSource::RemoteLocal => return Ok(()),
            ClientShellKeybindingSource::Local => {
                let mut config = crate::config::Config {
                    keys: self.local_keys.clone(),
                    ..Default::default()
                };
                config.keys.command = commands
                    .iter()
                    .filter_map(|command| {
                        let action_type = match command.action {
                            crate::protocol::ClientShellCommandAction::Shell => {
                                crate::config::CommandKeybindType::Shell
                            }
                            crate::protocol::ClientShellCommandAction::Pane => {
                                crate::config::CommandKeybindType::Pane
                            }
                            crate::protocol::ClientShellCommandAction::Popup => {
                                crate::config::CommandKeybindType::Popup
                            }
                            crate::protocol::ClientShellCommandAction::PluginAction => {
                                crate::config::CommandKeybindType::PluginAction
                            }
                            crate::protocol::ClientShellCommandAction::Unknown => return None,
                        };
                        Some(crate::config::CommandKeybindConfig {
                            key: if command.binding_labels.len() == 1 {
                                crate::config::BindingConfig::One(command.binding_labels[0].clone())
                            } else {
                                crate::config::BindingConfig::Many(command.binding_labels.clone())
                            },
                            // The client never executes this field; preserve the opaque endpoint ID
                            // through the shared config collision resolver.
                            command: command.command_id.clone(),
                            action_type,
                            description: command.description.clone(),
                            width: None,
                            height: None,
                        })
                    })
                    .collect();
                config
                    .live_keybinds_with_diagnostics()
                    .map(|(keybinds, _diagnostics)| keybinds)
                    .map_err(|diagnostics| diagnostics.join("; "))?
            }
        };
        if self.keybinding_source == ClientShellKeybindingSource::Endpoint {
            for command in commands {
                let Ok(action) = command.action.try_into() else {
                    continue;
                };
                keybinds
                    .keybinds
                    .custom_commands
                    .push(crate::config::CustomCommandKeybind {
                        bindings: crate::config::ActionKeybinds::from_labels(
                            &command.binding_labels,
                        )?,
                        label: command.binding_label.clone(),
                        command: command.command_id.clone(),
                        action,
                        description: command.description.clone(),
                        width: None,
                        height: None,
                    });
            }
        }
        self.keybinds = keybinds;
        Ok(())
    }

    pub(super) fn apply_live_config(
        &mut self,
        config: &Config,
        load_diagnostics: &[String],
        invalid_sections: &[String],
    ) -> Vec<String> {
        let mut diagnostics = load_diagnostics.to_vec();
        let invalid_section =
            |section: &str| invalid_sections.iter().any(|invalid| invalid == section);

        if !invalid_section("keys")
            && self.keybinding_source != ClientShellKeybindingSource::Endpoint
        {
            match config.live_keybinds_with_diagnostics() {
                Ok((mut keybinds, keybind_diagnostics)) => {
                    self.local_keys = config.keys.clone();
                    if self.keybinding_source == ClientShellKeybindingSource::RemoteLocal {
                        keybinds.keybinds.custom_commands.clear();
                    }
                    self.keybinds = keybinds;
                    diagnostics.extend(keybind_diagnostics);
                }
                Err(keybind_diagnostics) => diagnostics.extend(
                    keybind_diagnostics
                        .into_iter()
                        .map(|diagnostic| format!("{diagnostic}; kept current keybinds")),
                ),
            }
        }

        if !invalid_section("ui") {
            if let Some(diagnostic) = config.invalid_sidebar_bounds_diagnostic() {
                diagnostics.push(format!("{diagnostic}; keeping previous [ui] settings"));
            } else {
                let ui = &config.ui;
                diagnostics.extend(ui.sound.diagnostics());
                self.sidebar_width = ui.sidebar_width;
                self.sidebar_min_width = ui.sidebar_min_width;
                self.sidebar_max_width = ui.sidebar_max_width;
                self.sidebar_collapsed_mode = ui.sidebar_collapsed_mode;
                self.mobile_width_threshold = ui.mobile_width_threshold;
                self.tab_bar_position = ui.tab_bar_position;
                self.hide_tab_bar_when_single_tab = ui.hide_tab_bar_when_single_tab;
                self.spaces = ui.sidebar.spaces.clone();
                self.agents = ui.sidebar.agents.clone();
                self.agent_panel_sort = ui.agent_panel_sort;
                self.organization = ui.organization.clone();
                self.organization_locale = self.organization.locale();
                self.status_indicators = ui.status_indicators;
                self.sound_enabled = ui.sound.enabled;
                self.toast_delivery = ui.toast.delivery;
                self.toast_delay_seconds = ui.toast.delay_seconds;
                self.toast_position = ui.toast.herdr.position;
                self.copy_on_select = ui.copy_on_select;
                self.clipboard_toast_enabled = ui.toast.clipboard.enabled;
                self.clipboard_toast_position = ui.toast.clipboard.position;
                self.prompt_new_tab_name = ui.prompt_new_tab_name;
                self.prompt_new_workspace_name = ui.prompt_new_workspace_name;
                self.confirm_close = ui.confirm_close;
                self.mouse_capture = ui.mouse_capture;
                self.mouse_scroll_lines = ui.mouse_scroll_lines();
                self.right_click_passthrough_modifiers = ui.right_click_passthrough_modifiers();
                self.redraw_on_focus_gained = ui.redraw_on_focus_gained;
            }
        }

        if !invalid_section("theme") {
            self.theme_runtime = crate::app::client_theme_runtime_from_config(config);
            self.theme_name = self.theme_runtime.manual_name.clone();
            self.palette = crate::app::client_palette_from_config(config);
        }
        if !invalid_section("experimental") {
            self.switch_ascii_input_source_in_prefix =
                config.experimental.switch_ascii_input_source_in_prefix;
        }

        diagnostics
    }

    pub(super) fn layout(
        &self,
        cols: u16,
        rows: u16,
        sidebar_collapsed: bool,
        tab_count: usize,
        sidebar_width: u16,
    ) -> ClientShellLayout {
        if cols <= self.mobile_width_threshold {
            let header_height = rows.min(2);
            return ClientShellLayout {
                sidebar: Rect::default(),
                tab_bar: Rect::default(),
                mobile_header: Rect::new(0, 0, cols, header_height),
                pane_surface: Rect::new(0, header_height, cols, rows.saturating_sub(header_height)),
            };
        }

        let sidebar_width = if sidebar_collapsed {
            match self.sidebar_collapsed_mode {
                SidebarCollapsedModeConfig::Compact => 4,
                SidebarCollapsedModeConfig::Hidden => 0,
            }
        } else {
            let (min, max) = crate::config::validated_sidebar_bounds(
                self.sidebar_min_width,
                self.sidebar_max_width,
            )
            .unwrap_or((18, 36));
            sidebar_width.clamp(min, max)
        }
        .min(cols.saturating_sub(1));
        let main = Rect::new(sidebar_width, 0, cols.saturating_sub(sidebar_width), rows);
        let show_tab_bar = rows > 1 && !(self.hide_tab_bar_when_single_tab && tab_count == 1);
        let tab_height = u16::from(show_tab_bar);
        let (tab_bar, pane_surface) = match self.tab_bar_position {
            TabBarPositionConfig::Top => (
                Rect::new(main.x, 0, main.width, tab_height),
                Rect::new(
                    main.x,
                    tab_height,
                    main.width,
                    rows.saturating_sub(tab_height),
                ),
            ),
            TabBarPositionConfig::Bottom => (
                Rect::new(
                    main.x,
                    rows.saturating_sub(tab_height),
                    main.width,
                    tab_height,
                ),
                Rect::new(main.x, 0, main.width, rows.saturating_sub(tab_height)),
            ),
        };

        ClientShellLayout {
            sidebar: Rect::new(0, 0, sidebar_width, rows),
            tab_bar,
            mobile_header: Rect::default(),
            pane_surface,
        }
    }

    pub(crate) fn initial_surface_size(&self, cols: u16, rows: u16) -> ClientSurfaceSize {
        let sidebar_collapsed = self
            .preferences
            .sidebar_collapsed
            .unwrap_or(self.sidebar_start_collapsed);
        let (min_width, max_width) =
            crate::config::validated_sidebar_bounds(self.sidebar_min_width, self.sidebar_max_width)
                .unwrap_or((18, 36));
        let sidebar_width = self
            .preferences
            .sidebar_width
            .unwrap_or(self.sidebar_width)
            .clamp(min_width, max_width);
        let surface = self
            .layout(cols, rows, sidebar_collapsed, 0, sidebar_width)
            .pane_surface;
        ClientSurfaceSize {
            cols: surface.width.max(1),
            rows: surface.height.max(1),
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyModifiers};

    use super::*;

    #[test]
    fn live_reload_applies_client_owned_sections() {
        let mut shell = ClientShellConfig::from_config(&Config::default());
        let mut next = Config::default();
        next.ui.sidebar_width = 31;
        next.ui.tab_bar_position = TabBarPositionConfig::Bottom;
        next.ui.agent_panel_sort = crate::config::AgentPanelSortConfig::Priority;
        next.ui.status_indicators = crate::config::StatusIndicatorStyle::Symbols;
        next.ui.sidebar.agents = toml::from_str("rows = [[{ token = 'machine', rules = [{ equals = 'Local', bold = true }] }]]\nrow_gap = 2").unwrap();
        next.ui.organization.enabled = true;
        next.ui.organization.agents = vec![crate::config::OrganizationAgentConfig {
            id: "new-worker".into(),
            label: "Novo funcionário".into(),
            parent_id: Some("manager".into()),
            role: "worker".into(),
            ..Default::default()
        }];
        next.keys.prefix = "ctrl+a".to_owned();

        let diagnostics = shell.apply_live_config(&next, &[], &[]);

        assert!(diagnostics.is_empty());
        assert_eq!(shell.sidebar_width, 31);
        assert_eq!(shell.tab_bar_position, TabBarPositionConfig::Bottom);
        assert_eq!(
            shell.agent_panel_sort,
            crate::config::AgentPanelSortConfig::Priority
        );
        assert_eq!(
            shell.status_indicators,
            crate::config::StatusIndicatorStyle::Symbols
        );
        assert_eq!(shell.agents.row_gap, 2);
        assert!(shell.organization.enabled);
        assert_eq!(shell.organization.agents[0].id, "new-worker");
        assert_eq!(
            shell.organization.agents[0].parent_id.as_deref(),
            Some("manager")
        );
        assert_eq!(
            shell.agents.rows[0][0].style_for_value("Local").bold,
            Some(true)
        );
        let previous = shell.agents.clone();
        shell.apply_live_config(
            &Config::default(),
            &[],
            &["ui".to_owned(), "keys".to_owned()],
        );
        assert_eq!(shell.agents, previous);
        assert_eq!(
            shell.keybinds.prefix,
            (KeyCode::Char('a'), KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn initial_surface_size_uses_persisted_endpoint_chrome() {
        let path = std::env::temp_dir().join(format!(
            "herdr-initial-shell-preferences-{}.json",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        preferences::store(
            &path,
            preferences::ClientChromePreferences {
                sidebar_width: Some(31),
                sidebar_collapsed: Some(true),
                ..preferences::ClientChromePreferences::default()
            },
        )
        .expect("persist endpoint chrome");
        let config =
            ClientShellConfig::from_config(&Config::default()).with_preferences_path(path.clone());
        let initial = config.initial_surface_size(100, 30);
        let state = ClientShellState::new(config);
        assert_eq!(initial, state.surface_size(100, 30));
        std::fs::remove_file(path).expect("remove endpoint chrome");
    }

    #[test]
    fn live_reload_preserves_invalid_client_owned_sections() {
        let mut initial = Config::default();
        initial.ui.sidebar_width = 29;
        initial.keys.prefix = "ctrl+x".to_owned();
        let mut shell = ClientShellConfig::from_config(&initial);

        let mut invalid = Config::default();
        invalid.ui.sidebar_width = 35;
        invalid.keys.prefix = "ctrl+a".to_owned();
        let invalid_sections = vec!["ui".to_owned(), "keys".to_owned()];
        shell.apply_live_config(&invalid, &[], &invalid_sections);

        assert_eq!(shell.sidebar_width, 29);
        assert_eq!(
            shell.keybinds.prefix,
            (KeyCode::Char('x'), KeyModifiers::CONTROL)
        );
    }

    #[test]
    fn organization_registry_relative_path_uses_local_config_directory() {
        let root = std::env::temp_dir().join(format!(
            "herdr-org-relative-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let filename = format!("registry-{}.json", std::process::id());
        let registry = root.join(&filename);
        let config_file = root.join("config.toml");
        std::fs::write(&config_file, "# isolated config location").unwrap();
        std::fs::write(&registry,
            r#"{"schema_version":1,"agents":[{"id":"manager","role":"manager","pane_id":"pane-manager"},{"id":"worker","role":"worker","parent_id":"manager","pane_id":"pane-worker"}]}"#,
        ).unwrap();
        assert!(
            !std::path::Path::new(&filename).exists(),
            "fixture is not in process cwd"
        );
        let mut config = Config::default();
        config.ui.organization.enabled = true;
        config.ui.organization.registry_path = Some(filename);
        let mut shell_config = ClientShellConfig::from_config(&config);
        shell_config.local_config_path = config_file.clone();
        let mut state = ClientShellState::new(shell_config);
        assert!(state.endpoint_error.is_none(), "{:?}", state.endpoint_error);
        assert_eq!(state.config.organization.agents.len(), 2);
        assert_eq!(
            state.config.organization.agents[1].parent_id.as_deref(),
            Some("manager")
        );
        assert_eq!(
            state.config.organization.agents[1].pane_id.as_deref(),
            Some("pane-worker")
        );
        std::fs::write(&registry,
            r#"{"schema_version":1,"agents":[{"id":"worker","display_name":"Renamed worker","role":"worker","pane_id":"pane-worker"}]}"#,
        ).unwrap();
        assert!(state.tick_organization_registry(std::time::Instant::now()));
        assert_eq!(
            state.config.organization.agents[0].pane_id.as_deref(),
            Some("pane-worker")
        );
        assert_eq!(
            state.config.organization.agents[0].display_name.as_deref(),
            Some("Renamed worker")
        );
        std::fs::remove_file(registry).unwrap();
        std::fs::remove_file(config_file).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn organization_registry_reload_is_cached_and_keeps_last_valid_read() {
        let path = std::env::temp_dir().join(format!(
            "herdr-organization-registry-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        let mut config = Config::default();
        config.ui.organization.enabled = true;
        config.ui.organization.registry_path = Some(path.display().to_string());
        let mut shell = ClientShellState::new(ClientShellConfig::from_config(&config));
        std::fs::write(
            &path,
            r#"{
                "schema_version": 1,
                "agents": [{
                    "id": "worker",
                    "name": "Interface",
                    "display_name": "Interface",
                    "pane_id": "w1:p1",
                    "role": "worker",
                    "function": "Interface nativa",
                    "task": {"summary": "Implementar a interface", "source": "brief:test"}
                }]
            }"#,
        )
        .expect("write valid registry");
        assert!(shell.tick_organization_registry(std::time::Instant::now()));
        assert_eq!(shell.config.organization.agents.len(), 1);
        assert_eq!(
            shell.config.organization.agents[0].display_name.as_deref(),
            Some("Interface")
        );
        assert_eq!(
            shell.config.organization.agents[0].name.as_deref(),
            Some("Interface")
        );
        assert_eq!(
            shell.config.organization.agents[0].task.as_deref(),
            Some("Implementar a interface")
        );
        assert!(shell.endpoint_error.is_none());

        std::fs::write(
            &path,
            r#"{
                "schema_version": 1,
                "agents": [{"id": "worker-2", "display_name": "Build"}]
            }"#,
        )
        .expect("write changed registry");
        assert!(shell.tick_organization_registry(
            std::time::Instant::now() + std::time::Duration::from_secs(1)
        ));
        assert_eq!(shell.config.organization.agents[0].id, "worker-2");

        std::fs::write(&path, "{\"schema_version\": 1, \"agents\": [")
            .expect("write partial registry");
        assert!(shell.tick_organization_registry(
            std::time::Instant::now() + std::time::Duration::from_secs(2)
        ));
        assert_eq!(shell.config.organization.agents[0].id, "worker-2");
        assert!(shell
            .endpoint_error
            .as_deref()
            .is_some_and(|error| error.contains("organization registry invalid")));
        std::fs::write(
            &path,
            r#"{
                "schema_version": 1,
                "agents": [{"id": "worker-3", "display_name": "Recovered"}]
            }"#,
        )
        .expect("write recovered registry");
        assert!(shell.tick_organization_registry(
            std::time::Instant::now() + std::time::Duration::from_secs(3)
        ));
        assert_eq!(shell.config.organization.agents[0].id, "worker-3");
        assert!(shell.endpoint_error.is_none());

        shell.endpoint_error = Some("endpoint offline".into());
        std::fs::write(
            &path,
            r#"{
                "schema_version": 1,
                "agents": [{"id": "worker-4", "display_name": "Still available"}]
            }"#,
        )
        .expect("write registry beside endpoint error");
        assert!(shell.tick_organization_registry(
            std::time::Instant::now() + std::time::Duration::from_secs(4)
        ));
        assert_eq!(shell.endpoint_error.as_deref(), Some("endpoint offline"));
        std::fs::remove_file(path).expect("remove registry fixture");
    }
}
