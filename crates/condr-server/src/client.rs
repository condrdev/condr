//! The desktop's Client: `condr-client`'s connection over the transports this crate adds.

use crate::EndpointStream;

pub use condr_client::Refused;

/// A connection over any desktop transport; `ClientConnection::connect(&endpoint, …)`.
pub type ClientConnection = condr_client::ClientConnection<EndpointStream>;
