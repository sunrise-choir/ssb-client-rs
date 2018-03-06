//! The core ssb rpcs.

use futures::prelude::*;
use tokio_io::{AsyncRead, AsyncWrite};
use muxrpc::{InSyncResponse, ConnectionRpcError};
use serde_json::Value;
use ssb_rpc::ssb::{Whoami as RpcWhoami, WhoamiResponse};

use super::{Client, SendRpc};

pub fn whoami<R: AsyncRead, W: AsyncWrite>(client: &mut Client<R, W>) -> (SendRpc<W>, Whoami<R>) {
    lazy_static! {
       static ref WHOAMI: RpcWhoami = RpcWhoami::new();
   }

    let (req, res) = client
        .0
        .sync::<RpcWhoami, WhoamiResponse, Value>(&WHOAMI);
    (SendRpc::new_sync(req), Whoami(res))
}

/// Query information about the current user.
pub struct Whoami<R: AsyncRead>(InSyncResponse<R, WhoamiResponse, Value>);

impl<R: AsyncRead> Future for Whoami<R> {
    type Item = WhoamiResponse;
    type Error = ConnectionRpcError<Value>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll()
    }
}
