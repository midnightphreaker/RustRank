use rmcp::{
    ErrorData, RoleServer,
    model::{ClientJsonRpcMessage, ClientRequest, ErrorCode, ServerJsonRpcMessage},
    transport::Transport,
};

/// Let modern clients fall back to the legacy initialize handshake on the same
/// stdio connection. rmcp 1.7 otherwise exits on a pre-initialize discovery probe.
/// Do not advertise modern protocol support: the rest of this server uses the
/// legacy protocol and clients must still initialize before calling tools.
pub(super) struct LegacyDiscoveryTransport<T>(pub T);

impl<T: Transport<RoleServer>> Transport<RoleServer> for LegacyDiscoveryTransport<T> {
    type Error = T::Error;

    fn send(
        &mut self,
        message: ServerJsonRpcMessage,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        self.0.send(message)
    }

    async fn receive(&mut self) -> Option<ClientJsonRpcMessage> {
        loop {
            let message = self.0.receive().await?;
            if let ClientJsonRpcMessage::Request(request) = &message
                && let ClientRequest::CustomRequest(custom) = &request.request
                && custom.method == "server/discover"
            {
                let response = ServerJsonRpcMessage::error(
                    ErrorData::new(
                        ErrorCode::METHOD_NOT_FOUND,
                        "server/discover is not supported; use initialize",
                        None,
                    ),
                    Some(request.id.clone()),
                );
                if self.0.send(response).await.is_err() {
                    return None;
                }
                continue;
            }
            return Some(message);
        }
    }

    fn close(&mut self) -> impl Future<Output = Result<(), Self::Error>> + Send {
        self.0.close()
    }
}
