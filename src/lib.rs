//! A client library for interfacing with ssb.
// TODO example

#![deny(missing_docs)]

#[macro_use]
extern crate futures;
extern crate muxrpc;
extern crate ssb_common;
extern crate ssb_rpc;
extern crate tokio_io;
#[macro_use]
extern crate lazy_static;
extern crate serde_json;

use std::io;

use futures::prelude::*;
use futures::unsync::oneshot::Canceled;
use muxrpc::{muxrpc, RpcIn, RpcOut, Closed as RpcClosed, CloseRpc, RpcError, OutSync};
use tokio_io::{AsyncRead, AsyncWrite};

#[cfg(feature = "ssb")]
mod ssb;
#[cfg(feature = "ssb")]
pub use ssb::Whoami;

/// Take ownership of an AsyncRead and an AsyncWrite to create an ssb client.
// TODO example
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

    #[cfg(feature = "ssb")]
    /// Query information about the current user.
    pub fn whoami(&mut self) -> (SendRpc<W>, Whoami<R>) {
        ssb::whoami(self)
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
}

impl<W: AsyncWrite> Future for SendRpc<W> {
    type Item = ();
    /// `Some(err)` signals that a fatal io error occured when trying to send this rpc. `None`
    /// signals that a fatal io error happend upon sending another rpc.
    type Error = Option<io::Error>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        match self.0 {
            _SendRpc::Sync(ref mut out_sync) => out_sync.poll(),
        }
    }
}

enum _SendRpc<W: AsyncWrite> {
    Sync(OutSync<W>),
}
