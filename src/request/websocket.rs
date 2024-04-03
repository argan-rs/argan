use std::{
	borrow::Cow,
	future::{ready, Future},
	io::Error as IoError,
	pin::Pin,
	task::{Context, Poll},
};

use argan_core::BoxedError;
use base64::prelude::*;
use bytes::Bytes;
use fastwebsockets::{
	FragmentCollector, Frame, OpCode, Payload, Role, WebSocket as FastWebSocket,
	WebSocketError as FastWebSocketError,
};
use futures_util::FutureExt;
use http::{
	header::{
		CONNECTION, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_PROTOCOL,
		SEC_WEBSOCKET_VERSION, UPGRADE,
	},
	HeaderValue, Method,
};
use http_body_util::Empty;
use hyper::upgrade::{OnUpgrade, Upgraded};
use hyper_util::rt::TokioIo;
use sha1::{Digest, Sha1};

use crate::data::header::{split_header_value, HeaderMapExt};

use super::*;

// --------------------------------------------------------------------------------
// --------------------------------------------------------------------------------

// TODO: If non-utf-8 data within a text message send 1007.

// --------------------------------------------------
// WebSocketUpgrade

pub struct WebSocketUpgrade {
	response: Response,
	upgrade_future: UpgradeFuture,
	some_requested_protocols: Option<HeaderValue>,
	some_selected_protocol: Option<HeaderValue>,
	message_size_limit: usize,
	auto_unmasking: bool,
	auto_sending_pong: bool,
	auto_closing: bool,
}

impl WebSocketUpgrade {
	fn new(
		response: Response,
		upgrade_future: UpgradeFuture,
		some_requested_protocols: Option<HeaderValue>,
	) -> Self {
		Self {
			response,
			upgrade_future,
			some_requested_protocols,
			some_selected_protocol: None,
			message_size_limit: 16 * 1024 * 1024,
			auto_unmasking: true,
			auto_sending_pong: true,
			auto_closing: false,
		}
	}

	pub fn select_protocol<Func>(
		&mut self,
		selector: Func,
	) -> Result<Option<Cow<str>>, WebSocketUpgradeError>
	where
		Func: Fn(&str) -> bool,
	{
		if let Some(requested_protocols) = self.some_requested_protocols.as_ref() {
			let header_values = split_header_value(requested_protocols)
				.map_err(WebSocketUpgradeError::InvalidSecWebSocketProtocol)?;

			for header_value_str in header_values {
				if selector(header_value_str) {
					let header_value = HeaderValue::from_str(header_value_str)
						.expect("protocol header value should be taken from a valid header value");

					self.some_selected_protocol = Some(header_value);

					return Ok(Some(header_value_str.into()));
				}
			}
		}

		Ok(None)
	}

	pub fn set_message_size_limit(&mut self, size_limit: usize) -> &mut Self {
		self.message_size_limit = size_limit;

		self
	}

	pub fn turn_off_auto_unmasking(&mut self) -> &mut Self {
		self.auto_unmasking = false;

		self
	}

	pub fn turn_off_auto_sending_pong(&mut self) -> &mut Self {
		self.auto_sending_pong = false;

		self
	}

	pub fn turn_on_auto_closing(&mut self) -> &mut Self {
		self.auto_closing = true;

		self
	}

	pub fn upgrade<Func, Fut>(self, handle_upgrade_result: Func) -> Response
	where
		Func: FnOnce(Result<WebSocket, WebSocketUpgradeError>) -> Fut + Send + 'static,
		Fut: Future<Output = ()>,
	{
		let Self {
			mut response,
			upgrade_future,
			some_requested_protocols: requested_protocols,
			some_selected_protocol,
			message_size_limit,
			auto_unmasking,
			auto_sending_pong,
			auto_closing,
		} = self;

		tokio::spawn(async move {
			let result = upgrade_future.await.map(|mut fws| {
				fws.set_max_message_size(message_size_limit);
				fws.set_auto_apply_mask(auto_unmasking);
				fws.set_auto_pong(auto_sending_pong);
				fws.set_auto_close(auto_closing);

				WebSocket(FragmentCollector::new(fws))
			});

			handle_upgrade_result(result);
		});

		if let Some(selected_protocol) = some_selected_protocol {
			response
				.headers_mut()
				.insert(SEC_WEBSOCKET_PROTOCOL, selected_protocol);
		}

		response
	}
}

impl<B> FromRequest<B> for WebSocketUpgrade {
	type Error = WebSocketUpgradeError;

	fn from_request(mut request: Request<B>) -> impl Future<Output = Result<Self, Self::Error>> {
		ready(request_into_websocket_upgrade(request))
	}
}

pub(crate) fn request_into_websocket_upgrade<B>(
	mut request: Request<B>,
) -> Result<WebSocketUpgrade, WebSocketUpgradeError> {
	if request.method() != Method::GET {
		panic!("WebSocket is not supported with methods other than GET")
	}

	if !request
		.headers()
		.get(CONNECTION)
		.is_some_and(|header_value| header_value.as_bytes().eq_ignore_ascii_case(b"upgrade"))
	{
		return Err(WebSocketUpgradeError::InvalidConnectionHeader);
	}

	if !request
		.headers()
		.get(UPGRADE)
		.is_some_and(|header_value| header_value.as_bytes().eq_ignore_ascii_case(b"websocket"))
	{
		return Err(WebSocketUpgradeError::InvalidUpgradeHeader);
	}

	if !request
		.headers()
		.get(SEC_WEBSOCKET_VERSION)
		.is_some_and(|header_value| header_value.as_bytes() == b"13")
	{
		return Err(WebSocketUpgradeError::InvalidSecWebSocketVersion);
	}

	let Some(mut sec_websocket_accept_value) = request
		.headers()
		.get(SEC_WEBSOCKET_KEY)
		.map(|header_value| sec_websocket_accept_value_from(header_value.as_bytes()))
	else {
		return Err(WebSocketUpgradeError::MissingSecWebSocketKey);
	};

	let Some(upgrade_future) = request
		.extensions_mut()
		.remove::<OnUpgrade>()
		.map(UpgradeFuture)
	else {
		return Err(WebSocketUpgradeError::UnupgradableConnection);
	};

	let some_requested_protocols = request.headers_mut().remove(SEC_WEBSOCKET_PROTOCOL);

	let mut response = StatusCode::SWITCHING_PROTOCOLS.into_response();

	response
		.headers_mut()
		.insert(CONNECTION, HeaderValue::from_static("upgrade"));

	response
		.headers_mut()
		.insert(UPGRADE, HeaderValue::from_static("websocket"));

	response
		.headers_mut()
		.insert(SEC_WEBSOCKET_ACCEPT, sec_websocket_accept_value);

	Ok(WebSocketUpgrade::new(
		response,
		upgrade_future,
		some_requested_protocols,
	))
}

fn sec_websocket_accept_value_from(key: &[u8]) -> HeaderValue {
	let mut sha1 = Sha1::new();
	sha1.update(key);
	sha1.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");

	let b64 = BASE64_STANDARD.encode(sha1.finalize());
	HeaderValue::try_from(b64).expect("base64 encoded value must be a valid header value")
}

// --------------------------------------------------
// WebSocketUpgradeError

#[derive(Debug, crate::ImplError)]
pub enum WebSocketUpgradeError {
	#[error("invalid Connection header")]
	InvalidConnectionHeader,
	#[error("invalid Upgrade header")]
	InvalidUpgradeHeader,
	#[error("invalid Sec-WebSocket-Version")]
	InvalidSecWebSocketVersion,
	#[error("missing Sec-WebSocket-Key")]
	MissingSecWebSocketKey,
	#[error("invlaid Sec-WebSocket-Protocol")]
	InvalidSecWebSocketProtocol(ToStrError),
	#[error("unupgradable connection")]
	UnupgradableConnection,
	#[error(transparent)]
	Failure(#[from] hyper::Error),
}

impl IntoResponse for WebSocketUpgradeError {
	fn into_response(self) -> Response {
		use WebSocketUpgradeError::*;

		match self {
			InvalidConnectionHeader
			| InvalidUpgradeHeader
			| InvalidSecWebSocketVersion
			| MissingSecWebSocketKey
			| InvalidSecWebSocketProtocol(_) => StatusCode::BAD_REQUEST.into_response(),
			_ => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
		}
	}
}

// --------------------------------------------------
// UpgradeFuture

struct UpgradeFuture(OnUpgrade);

impl Future for UpgradeFuture {
	type Output = Result<FastWebSocket<TokioIo<Upgraded>>, WebSocketUpgradeError>;

	fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
		match self.0.poll_unpin(cx) {
			Poll::Ready(result) => Poll::Ready(
				result
					.map(|upgraded| FastWebSocket::after_handshake(TokioIo::new(upgraded), Role::Server))
					.map_err(WebSocketUpgradeError::Failure),
			),
			Poll::Pending => Poll::Pending,
		}
	}
}

// --------------------------------------------------
// WebSocket

pub struct WebSocket(FragmentCollector<TokioIo<Upgraded>>);

impl WebSocket {
	pub async fn receive(&mut self) -> Option<Result<Message, WebSocketError>> {
		match self.0.read_frame().await {
			Ok(complete_frame) => match complete_frame.opcode {
				OpCode::Text => {
					// Price of #![forbid(unsafe_code)]
					let text = String::from_utf8(complete_frame.payload.to_vec())
						.expect("text payload should have been guaranteed to be a valid utf-8");

					Some(Ok(Message::Text(text)))
				}
				OpCode::Binary => Some(Ok(Message::Binary(complete_frame.payload.to_vec()))),
				OpCode::Ping => Some(Ok(Message::Binary(complete_frame.payload.to_vec()))),
				OpCode::Pong => Some(Ok(Message::Binary(complete_frame.payload.to_vec()))),
				OpCode::Close => Some(Ok(Message::Close(None))),
				OpCode::Continuation => Some(Err(WebSocketError::Unexpected(IncompleteMessage.into()))),
			},
			Err(error) => {
				if let FastWebSocketError::ConnectionClosed = error {
					return None;
				}

				Some(Err(error.into()))
			}
		}
	}

	pub async fn send(&mut self, message: Message) -> Result<(), WebSocketError> {
		Ok(match message {
			Message::Text(text) => {
				let frame = Frame::text(Payload::Owned(text.into()));

				self.0.write_frame(frame).await?
			}
			Message::Binary(binary) => {
				let frame = Frame::binary(Payload::Owned(binary));

				self.0.write_frame(frame).await?
			}
			Message::Ping(ping) => {
				let frame = Frame::new(true, OpCode::Ping, None, Payload::Owned(ping));

				self.0.write_frame(frame).await?
			}
			Message::Pong(pong) => {
				let frame = Frame::pong(Payload::Owned(pong));

				self.0.write_frame(frame).await?
			}
			Message::Close(some_close_frame) => {
				let frame = if let Some(CloseFrame { code, reason }) = some_close_frame {
					Frame::close(code.into(), reason.as_bytes())
				} else {
					Frame::close(CloseCode::_1000_Normal.into(), b"")
				};

				self.0.write_frame(frame).await?
			}
		})
	}

	#[inline(always)]
	pub async fn close(mut self) -> Result<(), WebSocketError> {
		Ok(self.send(Message::Close(None)).await?)
	}
}

// --------------------------------------------------
// Message

pub enum Message {
	Text(String),
	Binary(Vec<u8>),
	Ping(Vec<u8>),
	Pong(Vec<u8>),
	Close(Option<CloseFrame>),
}

// ----------

pub struct CloseFrame {
	code: CloseCode,
	reason: Cow<'static, str>,
}

// --------------------------------------------------
// CloseCode

#[allow(non_camel_case_types)]
#[derive(Debug, Eq, PartialEq, Clone, Copy)]
pub enum CloseCode {
	/// Indicates a normal closure, meaning that the purpose for
	/// which the connection was established has been fulfilled.
	_1000_Normal,

	/// Indicates that an endpoint is "going away", such as a server
	/// going down or a browser having navigated away from a page.
	_1001_GoingAway,

	/// Indicates that an endpoint is terminating the connection due
	/// to a protocol error.
	_1002_ProtocolError,

	/// Indicates that an endpoint is terminating the connection
	/// because it has received a type of data it cannot accept (e.g., an
	/// endpoint that understands only text data MAY send this if it
	/// receives a binary message).
	_1003_UnsupportedData,

	/// Reserved. Indicates that no status code was included in a closing frame.
	/// This close code makes it possible to use a single method, `on_close` to
	/// handle even cases where no close code was provided.
	_1005_NoStatusReceived,

	/// Reserved. Indicates an abnormal closure. If the abnormal closure was due to
	/// an error, this close code will not be used. Instead, the `on_error` method
	/// of the handler will be called with the error. However, if the connection
	/// is simply dropped, without an error, this close code will be sent to the
	/// handler.
	_1006_Abnormal,

	/// Indicates that an endpoint is terminating the connection
	/// because it has received data within a message that was not
	/// consistent with the type of the message (e.g., non-UTF-8 \[RFC3629\]
	/// data within a text message).
	_1007_InvalidPayloadData,

	/// Indicates that an endpoint is terminating the connection
	/// because it has received a message that violates its policy.  This
	/// is a generic status code that can be returned when there is no
	/// other more suitable status code (e.g., Unsupported or Size) or if there
	/// is a need to hide specific details about the policy.
	_1008_PolicyViolation,

	/// Indicates that an endpoint is terminating the connection
	/// because it has received a message that is too big for it to
	/// process.
	_1009_MessageTooBig,

	/// Indicates that an endpoint (client) is terminating the
	/// connection because it has expected the server to negotiate one or
	/// more extension, but the server didn't return them in the response
	/// message of the WebSocket handshake.  The list of extensions that
	/// are needed should be given as the reason for closing.
	/// Note that this status code is not used by the server, because it
	/// can fail the WebSocket handshake instead.
	_1010_MandatoryExtension,

	/// Indicates that a server is terminating the connection because
	/// it encountered an unexpected condition that prevented it from
	/// fulfilling the request.
	_1011_InternalError,

	/// Indicates that the server is restarting. A client may choose to reconnect,
	/// and if it does, it should use a randomized delay of 5-30 seconds between attempts.
	_1012_ServerRestart,

	/// Indicates that the server is overloaded and the client should either connect
	/// to a different IP (when multiple targets exist), or reconnect to the same IP
	/// when a user has performed an action.
	_1013_TryLater,

	/// The server was acting as a gateway or proxy and received an invalid response
	/// from the upstream server.
	_1014_BadGateway,

	/// Reserved. Indicates that the connection was closed due to a failure to perform
	/// a TLS handshake (e.g., the server certificate can't be verified).
	_1015_TlsError,

	#[doc(hidden)]
	Unused(u16),
	#[doc(hidden)]
	Reserved(u16),
	#[doc(hidden)]
	IanaRegistered(u16),
	#[doc(hidden)]
	Private(u16),
	#[doc(hidden)]
	Bad(u16),
}

impl From<u16> for CloseCode {
	fn from(code: u16) -> CloseCode {
		use CloseCode::*;

		match code {
			1000 => _1000_Normal,
			1001 => _1001_GoingAway,
			1002 => _1002_ProtocolError,
			1003 => _1003_UnsupportedData,
			1005 => _1005_NoStatusReceived,
			1006 => _1006_Abnormal,
			1007 => _1007_InvalidPayloadData,
			1008 => _1008_PolicyViolation,
			1009 => _1009_MessageTooBig,
			1010 => _1010_MandatoryExtension,
			1011 => _1011_InternalError,
			1012 => _1012_ServerRestart,
			1013 => _1013_TryLater,
			1014 => _1014_BadGateway,
			1015 => _1015_TlsError,
			1..=999 => Unused(code),
			1016..=2999 => Reserved(code),
			3000..=3999 => IanaRegistered(code),
			4000..=4999 => Private(code),
			_ => Bad(code),
		}
	}
}

impl From<CloseCode> for u16 {
	fn from(code: CloseCode) -> u16 {
		use CloseCode::*;

		match code {
			_1000_Normal => 1000,
			_1001_GoingAway => 1001,
			_1002_ProtocolError => 1002,
			_1003_UnsupportedData => 1003,
			_1005_NoStatusReceived => 1005,
			_1006_Abnormal => 1006,
			_1007_InvalidPayloadData => 1007,
			_1008_PolicyViolation => 1008,
			_1009_MessageTooBig => 1009,
			_1010_MandatoryExtension => 1010,
			_1011_InternalError => 1011,
			_1012_ServerRestart => 1012,
			_1013_TryLater => 1013,
			_1014_BadGateway => 1014,
			_1015_TlsError => 1015,
			Unused(code) => code,
			Reserved(code) => code,
			IanaRegistered(code) => code,
			Private(code) => code,
			Bad(code) => code,
		}
	}
}

// --------------------------------------------------
// WebSocketError

#[non_exhaustive]
#[derive(Debug, crate::ImplError)]
pub enum WebSocketError {
	#[error("invalid fragment")]
	InvalidFragment,
	#[error("invalid UTF-8")]
	InvalidUTF8,
	#[error("invalid continuation frame")]
	InvalidContinuationFrame,
	#[error("invalid close frame")]
	InvalidCloseFrame,
	#[error("invalid close code")]
	InvalidCloseCode,
	#[error("unexpected EOF")]
	UnexpectedEOF,
	#[error("non-zero reserved bits")]
	NonZeroReservedBits,
	#[error("fragmented control frame")]
	FragmentedControlFrame,
	#[error("ping frame too large")]
	PingFrameTooLarge,
	#[error("message too large ")]
	MessageTooLarge,
	#[error("Invalid value")]
	InvalidValue,
	#[error(transparent)]
	IoError(#[from] IoError),
	#[error(transparent)]
	Unexpected(BoxedError),
}

impl From<FastWebSocketError> for WebSocketError {
	fn from(fast_web_socket_error: FastWebSocketError) -> Self {
		match fast_web_socket_error {
			FastWebSocketError::InvalidFragment => Self::InvalidFragment,
			FastWebSocketError::InvalidUTF8 => Self::InvalidUTF8,
			FastWebSocketError::InvalidContinuationFrame => Self::InvalidContinuationFrame,
			FastWebSocketError::InvalidCloseFrame => Self::InvalidCloseFrame,
			FastWebSocketError::InvalidCloseCode => Self::InvalidCloseCode,
			FastWebSocketError::UnexpectedEOF => Self::UnexpectedEOF,
			FastWebSocketError::ReservedBitsNotZero => Self::NonZeroReservedBits,
			FastWebSocketError::ControlFrameFragmented => Self::FragmentedControlFrame,
			FastWebSocketError::PingFrameTooLarge => Self::PingFrameTooLarge,
			FastWebSocketError::FrameTooLarge => Self::MessageTooLarge,
			FastWebSocketError::InvalidValue => Self::InvalidValue,
			FastWebSocketError::IoError(io_error) => Self::IoError(io_error),
			unexpected_error => Self::Unexpected(unexpected_error.into()),
		}
	}
}

#[derive(Debug, crate::ImplError)]
#[error("incomplete message")]
struct IncompleteMessage;

// --------------------------------------------------------------------------------