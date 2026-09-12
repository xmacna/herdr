use super::*;

/// State tracking for the thin client.
pub(super) struct ClientState {
    /// Stateful semantic-frame encoder used when the server sends FrameData.
    pub(super) blit_encoder: render_ansi::BlitEncoder,
    pub(super) mouse_capture_active: bool,
    pub(super) endpoint_mouse_capture_requested: bool,
    pub(super) endpoint_sgr_pixels_requested: bool,
    /// Latest physical host theme observations, retained so an endpoint selected after the
    /// observation receives the same client-owned baseline.
    pub(super) host_theme_updates: Vec<crate::protocol::ClientHostThemeUpdate>,
    pub(super) direct_mouse_capture_preference: bool,
    pub(super) shell_mouse_capture_preference: bool,
    pub(super) direct_keyboard_protocol: crate::terminal_modes::DirectHostKeyboardState,
    pub(super) pane_keyboard_report_all: bool,
    pub(super) keyboard_report_all_active: bool,
    pub(super) reported_size: (u16, u16),
    pub(super) reported_cell_size: (u32, u32),
    pub(super) sound_config: crate::config::SoundConfig,
    pub(super) kitty_graphics_enabled: bool,
    pub(super) pixel_geometry_enabled: bool,
    pub(super) pixel_geometry_exact: bool,
    #[cfg(unix)]
    pub(super) direct_graphics_response: Arc<Mutex<direct_graphics::ResponseMatcher>>,
    #[cfg(unix)]
    pub(super) retired_direct_graphics: Option<(endpoint::ClientEndpointId, u64, u32)>,
    #[cfg(unix)]
    pub(super) pending_surface_graphics:
        HashMap<(endpoint::ClientEndpointId, u64, u32), crate::protocol::SurfaceGraphicsAssetKey>,
    pub(super) attach_escape: Option<AttachEscapeState>,
    #[cfg(unix)]
    pub(super) mouse_scroll_lines: usize,
    pub(super) remote_image_paste_key:
        Option<(crossterm::event::KeyCode, crossterm::event::KeyModifiers)>,
    pub(super) redraw_on_focus_gained: bool,
    pub(super) repaint_pending: bool,
    /// During a source-off-first handoff the currently blitted frame remains authoritative until
    /// an acknowledged target snapshot/surface pair commits.
    pub(super) presentation_frozen: bool,
    pub(super) draw_host_cursor: bool,
    pub(super) detached_process_children: Vec<std::process::Child>,
    pub(super) integrated_presentation_sink:
        Option<Box<dyn crate::client::IntegratedPresentationSink>>,
    pub(super) integrated_presentation_error: Option<String>,
    pub(super) shell: Option<shell::ClientShellState>,
}

impl Drop for ClientState {
    fn drop(&mut self) {
        if self.attach_escape.is_some() {
            let _ = crate::terminal_modes::set_direct_host_keyboard_protocol(
                &mut io::stdout(),
                &mut self.direct_keyboard_protocol,
                0,
                0,
            );
        }
    }
}

impl ClientState {
    pub(crate) fn host_mode(&self) -> crate::client::host::HostMode {
        self.shell
            .as_ref()
            .map(shell::ClientShellState::host_mode)
            .unwrap_or(crate::client::host::HostMode::Ansi)
    }

    pub(crate) fn host_capabilities(&self) -> crate::client::host::HostCapabilities {
        self.shell
            .as_ref()
            .map(shell::ClientShellState::host_capabilities)
            .unwrap_or_else(crate::client::host::HostCapabilities::ansi)
    }

    pub(crate) fn set_host_mode(&mut self, mode: crate::client::host::HostMode) {
        if let Some(shell) = self.shell.as_mut() {
            shell.set_host_mode(mode);
        }
    }

    pub(super) fn set_integrated_presentation_sink(
        &mut self,
        sink: Option<Box<dyn crate::client::IntegratedPresentationSink>>,
    ) {
        let enabled = sink.is_some();
        self.integrated_presentation_sink = sink;
        if let Some(shell) = self.shell.as_mut() {
            shell.set_integrated_presentation_enabled(enabled);
        }
        if !enabled {
            self.integrated_presentation_error = None;
        }
    }

    fn present_integrated_full(&mut self, frame_data: &FrameData) {
        let Some(payload) = self
            .shell
            .as_mut()
            .and_then(|shell| shell.take_integrated_presentation())
        else {
            return;
        };
        let Some(sink) = self.integrated_presentation_sink.as_mut() else {
            return;
        };
        let frame = payload.with_frame(frame_data.clone());
        if let Err(error) = sink.present(crate::client::IntegratedPresentationUpdate::Full(&frame))
        {
            self.integrated_presentation_error = Some(error.to_string());
        }
    }

    pub(super) fn take_integrated_presentation_error(&mut self) -> Option<String> {
        self.integrated_presentation_error.take()
    }

    pub(super) fn request_repaint(&mut self) {
        self.repaint_pending = true;
    }

    pub(super) fn freeze_presentation(&mut self) {
        if !self.presentation_frozen {
            self.present_integrated_control(crate::client::IntegratedPresentationUpdate::Frozen);
        }
        self.presentation_frozen = true;
    }

    pub(super) fn record_host_theme_update(
        &mut self,
        update: &crate::protocol::ClientHostThemeUpdate,
    ) {
        use crate::protocol::ClientHostThemeUpdate;

        match update {
            ClientHostThemeUpdate::DefaultColor { kind, .. } => {
                self.host_theme_updates.retain(|current| {
                    !matches!(
                        current,
                        ClientHostThemeUpdate::DefaultColor {
                            kind: current_kind,
                            ..
                        } if current_kind == kind
                    )
                });
            }
            ClientHostThemeUpdate::PaletteColors(_) => self
                .host_theme_updates
                .retain(|current| !matches!(current, ClientHostThemeUpdate::PaletteColors(_))),
            ClientHostThemeUpdate::Appearance(_) => self
                .host_theme_updates
                .retain(|current| !matches!(current, ClientHostThemeUpdate::Appearance(_))),
        }
        self.host_theme_updates.push(update.clone());
    }

    /// Replay the retained physical-host baseline only after an endpoint owns the committed
    /// presentation. The endpoint transport preserves this order ahead of the resync control.
    pub(super) fn replay_host_theme(
        &self,
        endpoints: &mut endpoint::EndpointRegistry,
        endpoint_id: &endpoint::ClientEndpointId,
    ) {
        for update in &self.host_theme_updates {
            let _ = endpoints.send_to(
                endpoint_id,
                &crate::protocol::ClientMessage::ClientShellHostTheme {
                    update: update.clone(),
                },
            );
        }
    }

    pub(super) fn unfreeze_presentation(&mut self) {
        if self.presentation_frozen {
            self.present_integrated_control(crate::client::IntegratedPresentationUpdate::Thaw);
        }
        self.presentation_frozen = false;
        // A resize or metadata event may have happened while frozen. Force a full frame rather
        // than attempting to patch the old source frame.
        self.request_repaint();
    }

    /// Present a composed error/chrome frame while retaining the handoff input freeze. The pane
    /// cells are still the last coherent surface; only client chrome (including the error) moves.
    pub(super) fn present_frozen_chrome(&mut self, frame_data: FrameData) {
        let frozen = self.presentation_frozen;
        self.presentation_frozen = false;
        self.present_frame_internal(frame_data, !frozen);
        self.presentation_frozen = frozen;
    }

    pub(super) fn present_graphics(&mut self, graphics: &[u8]) {
        if self.presentation_frozen || graphics.is_empty() || !self.kitty_graphics_enabled {
            return;
        }
        if !self.host_capabilities().ansi_output {
            return;
        }
        let mut stdout = io::stdout();
        let _ = write_encoded_frame_with_graphics(&mut stdout, &[], graphics);
        let _ = stdout.flush();
    }

    pub(super) fn present_surface_patch(
        &mut self,
        patch: shell::ClientComposedSurfacePatch,
    ) -> io::Result<bool> {
        if !self.host_capabilities().ansi_output {
            return Ok(false);
        }
        if self.integrated_presentation_sink.is_some() {
            // The semantic package is produced during full composition. Force the existing
            // repaint path while the optional destination is enabled rather than sending a
            // patch without the matching organization revision.
            return Ok(false);
        }
        if self.presentation_frozen || self.repaint_pending {
            crate::render_prof::event("client_surface_patch.fallback.repaint");
            return Ok(false);
        }
        let draw_host_cursor = self.draw_host_cursor && self.host_capabilities().cursor;
        let rows = if draw_host_cursor {
            let Some(rows) = self
                .blit_encoder
                .patch_rows_with_drawn_cursor(&patch.rows, patch.cursor.as_ref())
            else {
                crate::render_prof::event("client_surface_patch.fallback.drawn_cursor");
                return Ok(false);
            };
            rows
        } else {
            patch.rows
        };
        let encode_started = crate::render_prof::timer();
        let Some(encoded) =
            self.blit_encoder
                .encode_patch(&rows, patch.cursor.clone(), draw_host_cursor)
        else {
            crate::render_prof::event("client_surface_patch.fallback.encode");
            return Ok(false);
        };
        crate::render_prof::duration_since("client_surface_patch.encode", encode_started);
        let write_started = crate::render_prof::timer();
        let mut stdout = io::stdout();
        stdout.write_all(&encoded.bytes)?;
        stdout.flush()?;
        crate::render_prof::duration_since("client_surface_patch.write", write_started);
        let committed = self.blit_encoder.commit_patch(&rows, patch.cursor, encoded);
        crate::render_prof::event(if committed {
            "client_surface_patch.success"
        } else {
            "client_surface_patch.fallback.commit"
        });
        Ok(committed)
    }

    #[cfg(unix)]
    pub(super) fn retire_endpoint_graphics(&mut self, endpoint_id: &endpoint::ClientEndpointId) {
        let transfer_ids = self
            .pending_surface_graphics
            .keys()
            .filter(|(owner, _, _)| owner == endpoint_id)
            .map(|(_, transfer_id, _)| *transfer_id)
            .collect::<Vec<_>>();
        self.pending_surface_graphics
            .retain(|(owner, _, _), _| owner != endpoint_id);
        if self
            .retired_direct_graphics
            .as_ref()
            .is_some_and(|(owner, _, _)| owner == endpoint_id)
        {
            self.retired_direct_graphics = None;
        }
        if let Ok(mut matcher) = self.direct_graphics_response.lock() {
            for transfer_id in transfer_ids {
                matcher.retire(transfer_id);
            }
        }
    }

    fn present_integrated_control(
        &mut self,
        update: crate::client::IntegratedPresentationUpdate<'_>,
    ) {
        let Some(sink) = self.integrated_presentation_sink.as_mut() else {
            return;
        };
        if let Err(error) = sink.present(update) {
            self.integrated_presentation_error = Some(error.to_string());
        }
    }

    pub(super) fn present_frame(&mut self, frame_data: FrameData) {
        self.present_frame_internal(frame_data, true);
    }

    fn present_frame_internal(&mut self, frame_data: FrameData, integrated: bool) {
        if self.presentation_frozen {
            return;
        }
        let capabilities = self.host_capabilities();
        if integrated && self.integrated_presentation_sink.is_some() {
            self.present_integrated_full(&frame_data);
        }
        if !capabilities.ansi_output {
            return;
        }
        let draw_host_cursor = self.draw_host_cursor && capabilities.cursor;
        let frame_data = if draw_host_cursor {
            render_ansi::frame_with_drawn_cursor(frame_data)
        } else {
            frame_data
        };
        let encoded = if draw_host_cursor {
            self.blit_encoder
                .encode_with_suppressed_visible_cursor(&frame_data, self.repaint_pending)
        } else {
            self.blit_encoder.encode(&frame_data, self.repaint_pending)
        };
        let mut stdout = io::stdout();
        let graphics = if self.kitty_graphics_enabled {
            frame_data.graphics.as_slice()
        } else {
            &[]
        };
        let _ = write_encoded_frame_with_graphics(&mut stdout, &encoded.bytes, graphics);
        let _ = stdout.flush();
        self.blit_encoder.commit(frame_data, encoded);
        self.repaint_pending = false;
    }
}
