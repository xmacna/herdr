use super::*;

/// Feed the platform stdin batch through the same shell dispatcher and finish path used by the
/// Windows client loop. The optional presentation sink is reached only after composition and
/// endpoint routing; this helper does not send pane input directly.
#[cfg(windows)]
pub(crate) fn integrated_client_event_to_presentation(
    state: &mut ClientState,
    events: &[crate::protocol::ClientInputEvent],
    write_stream: &mut endpoint::EndpointRegistry,
    pending_activation: &mut Option<endpoint::PendingEndpointActivation>,
    endpoint_commands: &mut endpoint_commands::EndpointCommands,
    prefix_input_source: &mut impl crate::platform::PrefixInputSource,
    event_tx: &tokio::sync::mpsc::Sender<ClientLoopEvent>,
) -> Result<bool, ClientError> {
    let (outcome, frame) = {
        let shell = state
            .shell
            .as_mut()
            .expect("integrated shell event requires shell");
        let outcome = shell.handle_client_events(events);
        let frame = outcome
            .repaint
            .then(|| shell.compose(state.reported_size.0, state.reported_size.1))
            .flatten();
        (outcome, frame)
    };
    finish_client_shell_input(
        state,
        outcome,
        frame,
        write_stream,
        pending_activation,
        endpoint_commands,
        prefix_input_source,
        event_tx,
    )
}
