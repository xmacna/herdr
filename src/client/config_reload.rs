use super::*;

pub(super) const CONFIG_POLL_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFileStamp {
    Present {
        modified: Option<std::time::SystemTime>,
        len: u64,
    },
    Unavailable(std::io::ErrorKind),
}

fn config_file_stamp(path: &std::path::Path) -> ConfigFileStamp {
    match std::fs::metadata(path) {
        Ok(metadata) => ConfigFileStamp::Present {
            modified: metadata.modified().ok(),
            len: metadata.len(),
        },
        Err(error) => ConfigFileStamp::Unavailable(error.kind()),
    }
}

pub(super) struct ClientConfigWatcher {
    path: std::path::PathBuf,
    stamp: ConfigFileStamp,
    next_poll: std::time::Instant,
}

impl ClientConfigWatcher {
    pub(super) fn new(path: std::path::PathBuf, now: std::time::Instant) -> Self {
        Self {
            stamp: config_file_stamp(&path),
            path,
            next_poll: now.checked_add(CONFIG_POLL_INTERVAL).unwrap_or(now),
        }
    }

    pub(super) fn changed(&mut self, now: std::time::Instant) -> bool {
        if now < self.next_poll {
            return false;
        }
        self.next_poll = now.checked_add(CONFIG_POLL_INTERVAL).unwrap_or(now);
        let stamp = config_file_stamp(&self.path);
        if stamp == self.stamp {
            return false;
        }
        self.stamp = stamp;
        true
    }
}

pub(super) fn init_logging() {
    crate::logging::init_file_logging("herdr-client.log");
}

pub(super) fn apply_reload(
    state: &mut ClientState,
    endpoints: &mut endpoint::EndpointRegistry,
    pending_activation: &mut Option<endpoint::PendingEndpointActivation>,
    host_mouse_capture_active: &std::sync::atomic::AtomicBool,
    host_sgr_pixels_active: &std::sync::atomic::AtomicBool,
    prefix_input_source: &mut impl crate::platform::PrefixInputSource,
) -> Result<(), ClientError> {
    let previous_mouse_capture = state.shell_mouse_capture_preference;
    let mut mouse_capture = previous_mouse_capture;
    reload_local_client_config(
        &mut state.sound_config,
        &mut state.redraw_on_focus_gained,
        &mut state.draw_host_cursor,
        &mut state.remote_image_paste_key,
        &mut mouse_capture,
    );
    state.shell_mouse_capture_preference = mouse_capture;
    state.direct_mouse_capture_preference = state.attach_escape.is_some() && mouse_capture;
    if state.shell.is_some() && previous_mouse_capture != mouse_capture {
        write_to_server(
            endpoints,
            &ClientMessage::ClientShellMouseCapture {
                enabled: mouse_capture,
            },
        )
        .map_err(ClientError::ConnectionLost)?;
    }
    if state.attach_escape.is_some() {
        let enabled = effective_mouse_capture(
            state.endpoint_mouse_capture_requested,
            state.direct_mouse_capture_preference,
        );
        let sgr_pixels = effective_sgr_pixel_mouse(
            enabled,
            state.endpoint_sgr_pixels_requested,
            state.pixel_geometry_exact,
        );
        if enabled != state.mouse_capture_active
            || sgr_pixels != host_sgr_pixels_active.load(Ordering::Acquire)
        {
            set_mouse_capture(enabled, sgr_pixels).map_err(ClientError::ConnectionFailed)?;
        }
        state.mouse_capture_active = enabled;
        host_mouse_capture_active.store(enabled, Ordering::Release);
        host_sgr_pixels_active.store(sgr_pixels, Ordering::Release);
    }
    let (frame, resize) = if let Some(shell) = state.shell.as_mut() {
        let previous_size = shell.surface_size(state.reported_size.0, state.reported_size.1);
        shell.reload_client_config();
        let next_size = shell.surface_size(state.reported_size.0, state.reported_size.1);
        let resize = (previous_size != next_size).then(|| {
            shell.invalidate_pane_surface();
            client_shell_resize_message(
                shell,
                state.reported_size.0,
                state.reported_size.1,
                state.reported_cell_size.0,
                state.reported_cell_size.1,
                state.pixel_geometry_exact,
            )
        });
        (
            shell.compose(state.reported_size.0, state.reported_size.1),
            resize,
        )
    } else {
        (None, None)
    };
    apply_client_shell_input_source_changes(state, prefix_input_source);
    if let Some(resize) = resize {
        if let Some(activation) = pending_activation.as_mut() {
            if let Err(error) = activation.update_resize(resize, endpoints) {
                rollback_endpoint_activation(state, endpoints, pending_activation, error, false);
            }
        } else {
            write_to_server(endpoints, &resize).map_err(ClientError::ConnectionLost)?;
        }
    }
    if let Some(frame) = frame {
        state.present_frame(frame);
    }
    Ok(())
}

pub(super) fn reload_local_client_config(
    sound_config: &mut crate::config::SoundConfig,
    redraw_on_focus_gained: &mut bool,
    draw_host_cursor: &mut bool,
    remote_image_paste_key: &mut Option<(
        crossterm::event::KeyCode,
        crossterm::event::KeyModifiers,
    )>,
    mouse_capture: &mut bool,
) {
    match crate::config::load_live_config() {
        Ok(loaded) => {
            let invalid_section = |section: &str| {
                loaded
                    .invalid_sections
                    .iter()
                    .any(|invalid| invalid == section)
            };
            if !invalid_section("ui") && loaded.config.invalid_sidebar_bounds_diagnostic().is_none()
            {
                for diagnostic in loaded.config.ui.sound.diagnostics() {
                    warn!(diagnostic = %diagnostic, "local sound config diagnostic");
                }
                *sound_config = loaded.config.ui.sound.clone();
                *redraw_on_focus_gained = loaded.config.ui.redraw_on_focus_gained;
                *draw_host_cursor = should_draw_host_cursor(loaded.config.ui.host_cursor);
                *mouse_capture = loaded.config.ui.mouse_capture;
            }
            if !invalid_section("keys") {
                *remote_image_paste_key = client_remote_image_paste_key(&loaded.config);
            }
            debug!("reloaded local client config");
        }
        Err(diagnostics) => {
            warn!(diagnostics = ?diagnostics, "failed to reload local client config; keeping current client config");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_watcher_reports_file_changes_only_after_the_poll_interval() {
        let path = std::env::temp_dir().join(format!(
            "herdr-client-config-watch-{}.toml",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let now = std::time::Instant::now();
        let mut watcher = ClientConfigWatcher::new(path.clone(), now);

        std::fs::write(&path, "[ui]\nsidebar_width = 31\n").expect("write watched config");
        assert!(!watcher.changed(now + Duration::from_secs(1)));
        assert!(watcher.changed(now + CONFIG_POLL_INTERVAL));
        assert!(!watcher.changed(now + CONFIG_POLL_INTERVAL + Duration::from_millis(1)));

        std::fs::write(&path, "[ui]\nsidebar_width = 44\n# changed length\n")
            .expect("update watched config");
        assert!(watcher.changed(now + CONFIG_POLL_INTERVAL * 2));
        std::fs::remove_file(path).expect("remove watched config");
    }
}
