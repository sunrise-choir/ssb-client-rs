//! The core ssb rpcs.

use futures::prelude::*;
use muxrpc::{ConnectionRpcError, InAsyncResponse, InSyncResponse};
use serde::de::DeserializeOwned;
use serde_json::Value;
use ssb_common::links::MessageId;
use ssb_common::messages::Message;
use ssb_rpc::ssb::{Get as RpcGet, Whoami as RpcWhoami, WhoamiResponse};
use tokio_io::{AsyncRead, AsyncWrite};

use super::{Client, SendRpc};

pub fn whoami<R: AsyncRead, W: AsyncWrite>(client: &mut Client<R, W>) -> (SendRpc<W>, Whoami<R>) {
    lazy_static! {
        static ref WHOAMI: RpcWhoami = RpcWhoami::new();
    }

    let (req, res) = client.0.sync::<RpcWhoami, WhoamiResponse, Value>(&WHOAMI);
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

pub fn get<R: AsyncRead, W: AsyncWrite, T: DeserializeOwned>(
    client: &mut Client<R, W>,
    id: MessageId,
) -> (SendRpc<W>, Get<R, T>) {
    let (req, res) = client
        .0
        .async::<RpcGet, Message<T>, Value>(&RpcGet::new(id));
    (SendRpc::new_async(req), Get(res))
}

/// Get a `Message` by its `MessageId`.
pub struct Get<R: AsyncRead, T: DeserializeOwned>(InAsyncResponse<R, Message<T>, Value>);

impl<R: AsyncRead, T: DeserializeOwned> Future for Get<R, T> {
    type Item = Message<T>;
    type Error = ConnectionRpcError<Value>;

    fn poll(&mut self) -> Poll<Self::Item, Self::Error> {
        self.0.poll()
    }
}
