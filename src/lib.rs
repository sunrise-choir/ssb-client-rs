//! A client library for interfacing with ssb.
//!
//! ```rust,ignore
//! sodiumoxide::init();
//! let addr = SocketAddr::new(Ipv6Addr::localhost().into(), DEFAULT_TCP_PORT);
//!
//! current_thread::run(|_| {
//!     current_thread::spawn(TcpStream::connect(&addr)
//!     .and_then(|tcp| easy_ssb(tcp).unwrap().map_err(|err| panic!("{:?}", err)))
//!     .map_err(|err| panic!("{:?}", err))
//!     .map(|(mut client, receive, _)| {
//!         current_thread::spawn(receive.map_err(|err| panic!("{:?}", err)));
//!
//!         let (send_request, response) = client.whoami();
//!
//!         current_thread::spawn(send_request.map_err(|err| panic!("{:?}", err)));
//!         current_thread::spawn(response
//!                                   .map(|res| println!("{:?}", res))
//!                                   .map_err(|err| panic!("{:?}", err))
//!                                   .and_then(|_| {
//!                                                 client.close().map_err(|err| panic!("{:?}", err))
//!                                             }));
//!     }))
//! });
//! ```

#![deny(missing_docs)]
#![feature(try_from)]
#![feature(ip_constructors)] // only for tests

#[macro_use]
extern crate futures;
extern crate muxrpc;
extern crate ssb_common;
extern crate ssb_rpc;
extern crate tokio_io;
#[macro_use]
extern crate lazy_static;
extern crate serde;
extern crate serde_json;
extern crate ssb_keyfile;
extern crate secret_stream;
extern crate secret_handshake;
extern crate box_stream;
extern crate sodiumoxide;
#[cfg(test)]
extern crate tokio;

use std::convert::{From, TryInto};
use std::io;

use futures::prelude::*;
use futures::future::Then;
use futures::unsync::oneshot::Canceled;
use muxrpc::{muxrpc, RpcIn, RpcOut, Closed as RpcClosed, CloseRpc, RpcError, OutSync, OutAsync};
use tokio_io::{AsyncRead, AsyncWrite};
use tokio_io::io::{ReadHalf, WriteHalf};
use ssb_keyfile::{KeyfileError, load_or_create_keys};
use secret_stream::OwningClient;
use ssb_common::MAINNET_IDENTIFIER;
use ssb_common::links::MessageId;
use sodiumoxide::crypto::box_;
use box_stream::BoxDuplex;
use secret_handshake::ClientHandshakeFailure;
use serde::de::DeserializeOwned;

#[cfg(feature = "ssb")]
mod ssb;
#[cfg(feature = "ssb")]
pub use ssb::{Whoami, Get};

/// Take ownership of an AsyncRead and an AsyncWrite to create an ssb client.
///
/// # Example
///
/// ```rust,ignore
/// sodiumoxide::init();
///
/// let (pk, sk) = load_or_create_keys().unwrap();
/// let pk = pk.try_into().unwrap();
/// let sk = sk.try_into().unwrap();
/// let (ephemeral_pk, ephemeral_sk) = box_::gen_keypair();
///
/// let addr = SocketAddr::new(Ipv6Addr::localhost().into(), DEFAULT_TCP_PORT);
///
/// current_thread::run(|_| {
///     current_thread::spawn(TcpStream::connect(&addr)
///                               .and_then(move |tcp| {
///         // Performs a secret-handshake and yields an encrypted duplex connection.
///         OwningClient::new(tcp,
///                           &MAINNET_IDENTIFIER,
///                           &pk, &sk,
///                           &ephemeral_pk, &ephemeral_sk,
///                           &pk)
///                 .map_err(|(err, _)| err)
///     })
///       .map_err(|err| panic!("{:?}", err))
///       .map(move |connection| {
///         let (read, write) = connection.unwrap().split();
///         let (mut client, receive, _) = ssb(read, write);
///         current_thread::spawn(receive.map_err(|err| panic!("{:?}", err)));
///
///         let (send_request, response) = client.whoami();
///
///         current_thread::spawn(send_request.map_err(|err| panic!("{:?}", err)));
///         current_thread::spawn(response
///                                   .map(|res| println!("{:?}", res))
///                                   .map_err(|err| panic!("{:?}", err))
///                                   .and_then(|_| client.close().map_err(|err| panic!("{:?}", err))));
///     }))
/// });
/// ```
pub fn ssb<R: AsyncRead, W: AsyncWrite>(r: R, w: W) -> (Client<R, W>, Receive<R, W>, Closed<W>) {
    let (rpc_in, rpc_out, rpc_closed) = muxrpc(r, w);
    (Client(rpc_out), Receive(rpc_in), Closed(rpc_closed))
}

/// An ssb client. This struct is used to send rpcs to the server.
pub struct Client<R: AsyncRead, W>(RpcOut<R, W>);

impl<R: AsyncRead, W: AsyncWrite> Client<R, W> {
    /// Close the connection to the server. If there are still active rpcs, it is not closed
    /// immediately. It will get closed once the last of them is done.
    pub fn close(self) -> Close<R, W> {
        Close(self.0.close())
    }

    /// Give access to the underlying muxrpc `RpcOut`, to send rpcs which are not directly
    /// supported by this module.
    pub fn muxrpc(&mut self) -> &mut RpcOut<R, W> {
        &mut self.0
    }

    #[cfg(feature = "ssb")]
    /// Query information about the current user.
    pub fn whoami(&mut self) -> (SendRpc<W>, Whoami<R>) {
        ssb::whoami(self)
    }

    #[cfg(feature = "ssb")]
    /// Get a `Message` of type `T` by its `MessageId`..
    pub fn get<T: DeserializeOwned>(&mut self, id: MessageId) -> (SendRpc<W>, Get<R, T>) {
        ssb::get(self, id)
    }
}

/// A future that has to be polled in order to receive responses from the server.
pub struct Receive<R: AsyncRead, W>(RpcIn<R, W>);

impl<R: AsyncRead, W: AsyncWrite> Future for Receive<R, W> {
    /// Yielded when the server properly terminates the connection.
    type Item = ();
    type Error = RpcError;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        while let Some(_) = try_ready!(self.0.poll()) {}
        Ok(Async::Ready(()))
    }
}

/// A future that indicates when the write-half of the channel to the server has been closed.
pub struct Closed<W>(RpcClosed<W>);

impl<W> Future for Closed<W> {
    type Item = W;
    /// This can only be emitted if some rpc future/sink/stream has been polled but was then
    /// dropped before it was done. If all handles are properly polled/closed, this is never
    /// emitted.
    type Error = Canceled;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll()
    }
}

/// A future for closing the connection to the server. If there are still active rpcs, it is not
/// closed immediately. It will get closed once the last of them is done.
pub struct Close<R: AsyncRead, W>(CloseRpc<R, W>);

impl<R: AsyncRead, W: AsyncWrite> Future for Close<R, W> {
    type Item = ();

    type Error = Option<io::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll()
    }
}

/// A future for sending an rpc to the server. If this isn't polled, the rpc is never sent.
pub struct SendRpc<W: AsyncWrite>(_SendRpc<W>);

impl<W: AsyncWrite> SendRpc<W> {
    fn new_sync(out_sync: OutSync<W>) -> SendRpc<W> {
        SendRpc(_SendRpc::Sync(out_sync))
    }

    fn new_async(out_async: OutAsync<W>) -> SendRpc<W> {
        SendRpc(_SendRpc::Async(out_async))
    }
}

impl<W: AsyncWrite> Future for SendRpc<W> {
    type Item = ();
    /// `Some(err)` signals that a fatal io error occured when trying to send this rpc. `None`
    /// signals that a fatal io error happend upon sending another rpc.
    type Error = Option<io::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.0 {
            _SendRpc::Sync(ref mut out_sync) => out_sync.poll(),
            _SendRpc::Async(ref mut out_async) => out_async.poll(),
        }
    }
}

enum _SendRpc<W: AsyncWrite> {
    Sync(OutSync<W>),
    Async(OutAsync<W>),
}

/// Return a future for setting up an encrypted connection via the given transport
/// (using keys from the ssb keyfile) and then calling `ssb` on the connection.
///
/// This function performs blocking file io (for reading the keyfile).
///
/// This function uses libsodium internally, so ensure that `sodiumoxide::init()` has been called
/// before using this function.
///
/// # Example
///
/// ```rust,ignore
/// sodiumoxide::init();
/// let addr = SocketAddr::new(Ipv6Addr::localhost().into(), DEFAULT_TCP_PORT);
///
/// current_thread::run(|_| {
///     current_thread::spawn(TcpStream::connect(&addr)
///     .and_then(|tcp| easy_ssb(tcp).unwrap().map_err(|err| panic!("{:?}", err)))
///     .map_err(|err| panic!("{:?}", err))
///     .map(|(mut client, receive, _)| {
///         current_thread::spawn(receive.map_err(|err| panic!("{:?}", err)));
///
///         let (send_request, response) = client.whoami();
///
///         current_thread::spawn(send_request.map_err(|err| panic!("{:?}", err)));
///         current_thread::spawn(response
///                                   .map(|res| println!("{:?}", res))
///                                   .map_err(|err| panic!("{:?}", err))
///                                   .and_then(|_| {
///                                                 client.close().map_err(|err| panic!("{:?}", err))
///                                             }));
///     }))
/// });
/// ```
pub fn easy_ssb<T: AsyncRead + AsyncWrite>(transport: T) -> Result<EasySsb<T>, KeyfileError> {
    let (pk, sk) = load_or_create_keys()?;
    let pk = pk.try_into().unwrap();
    let sk = sk.try_into().unwrap();
    let (ephemeral_pk, ephemeral_sk) = box_::gen_keypair();

    Ok(EasySsb::new(OwningClient::new(transport,
                                      MAINNET_IDENTIFIER.clone(),
                                      pk,
                                      sk,
                                      ephemeral_pk,
                                      ephemeral_sk,
                                      pk)))
}

type AR<T> = ReadHalf<BoxDuplex<T>>;
type AW<T> = WriteHalf<BoxDuplex<T>>;
type ClientTriple<T> = (Client<AR<T>, AW<T>>, Receive<AR<T>, AW<T>>, Closed<AW<T>>);

/// A future for setting up an encrypted connection via the given AsyncRead and AsyncWrite
/// (using keys from the ssb keyfile) and then calling `ssb` on the connection.
pub struct EasySsb<T: AsyncRead + AsyncWrite>(Then<OwningClient<T>,
                                                    Result<ClientTriple<T>, EasySsbError<T>>,
                                                    fn(Result<Result<BoxDuplex<T>,
                                                                     (ClientHandshakeFailure,
                                                                      T)>,
                                                              (io::Error, T)>)
                                                       -> Result<ClientTriple<T>,
                                                                  EasySsbError<T>>>);

impl<T: AsyncRead + AsyncWrite> EasySsb<T> {
    fn new(secret_client: OwningClient<T>) -> EasySsb<T> {
        EasySsb(secret_client.then(|res| match res {
                                       Ok(Ok(duplex)) => {
            let (read, write) = duplex.split();
            Ok(ssb(read, write))
        }
                                       Ok(Err((failure, transport))) => {
                                           Err(EasySsbError::FailedHandshake(failure, transport))
                                       }
                                       Err((err, transport)) => {
                                           Err(EasySsbError::IoError(err, transport))
                                       }
                                   }))
    }
}

impl<T: AsyncRead + AsyncWrite> Future for EasySsb<T> {
    type Item = (Client<AR<T>, AW<T>>, Receive<AR<T>, AW<T>>, Closed<AW<T>>);
    type Error = EasySsbError<T>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll()
    }
}

/// Everything that can go wrong when creating a client via `easy_ssb`.
#[derive(Debug)]
pub enum EasySsbError<T> {
    /// The handshake was performed without io errors but did not terminate successfully.
    FailedHandshake(ClientHandshakeFailure, T),
    /// An io error happened during the handshake.
    IoError(io::Error, T),
}

#[cfg(test)]
mod tests {
    use std::net::{SocketAddr, Ipv6Addr};

    use tokio::executor::current_thread;
    use tokio::net::TcpStream;
    use ssb_common::*;

    use super::*;

    #[test]
    fn test_easy_ssb() {
        sodiumoxide::init();
        let addr = SocketAddr::new(Ipv6Addr::localhost().into(), DEFAULT_TCP_PORT);

        current_thread::run(|_| {
            current_thread::spawn(TcpStream::connect(&addr)
            .and_then(|tcp| easy_ssb(tcp).unwrap().map_err(|err| panic!("{:?}", err)))
            .map_err(|err| panic!("{:?}", err))
            .map(|(mut client, receive, _)| {
                current_thread::spawn(receive.map_err(|err| panic!("{:?}", err)));

                let (send_request, response) = client.whoami();

                current_thread::spawn(send_request.map_err(|err| panic!("{:?}", err)));
                current_thread::spawn(response
                                          .map(|res| println!("{:?}", res))
                                          .map_err(|err| panic!("{:?}", err))
                                          .and_then(|_| {
                                                        client.close().map_err(|err| panic!("{:?}", err))
                                                    }));
            }))
        });
    }
}
