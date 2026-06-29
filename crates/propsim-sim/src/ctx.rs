//! [`HarnessCtx`] — the scheduler's implementation of the node effect surface
//! [`propsim_core::Ctx`].
//!
//! It borrows the [`SchedCore`] (everything *except* the node slice) so the
//! active node's `&mut N` and this `&mut dyn Ctx` never alias.

use std::time::Duration;

use propsim_core::history::{OpKind, Value};
use propsim_core::node::{ClientCodec, Completion, Ctx, Node, Rng};
use propsim_core::NodeId;

use crate::scheduler::SchedCore;

/// The per-callback context: a mutable borrow of the scheduler core, the id of
/// the node currently executing, and the client codec (needed to encode a
/// completion response back to a `Value` for the recorded `Ok`).
pub struct HarnessCtx<'a, N: Node>
where
    N::Timer: std::fmt::Debug,
    N::Msg: Clone,
{
    pub(crate) core: &'a mut SchedCore<N>,
    pub(crate) me: NodeId,
    pub(crate) codec: Option<&'a dyn ClientCodec<N>>,
}

impl<'a, N: Node> Ctx<N> for HarnessCtx<'a, N>
where
    N::Timer: std::fmt::Debug + Clone,
    N::Msg: Clone,
{
    fn send(&mut self, to: NodeId, msg: N::Msg) {
        self.core.ctx_send(self.me, to, msg);
    }

    fn broadcast(&mut self, msg: N::Msg)
    where
        N::Msg: Clone,
    {
        for peer in self.core.peers(self.me) {
            self.core.ctx_send(self.me, peer, msg.clone());
        }
    }

    fn set_timer(&mut self, timer: N::Timer, after: Duration) {
        self.core.ctx_set_timer(self.me, timer, after);
    }

    fn cancel_timer(&mut self, timer: N::Timer) {
        self.core.ctx_cancel_timer(self.me, &timer);
    }

    fn now(&self) -> Duration {
        self.core.now
    }

    fn rng(&mut self) -> &mut dyn Rng {
        &mut self.core.rng
    }

    fn me(&self) -> NodeId {
        self.me
    }

    fn complete_op(&mut self, token: propsim_core::OpToken, completion: Completion<N::Response>) {
        match completion {
            Completion::Ok(resp) => {
                // Encode the typed response into the Ok entry's value. The codec
                // is always present here: a token only exists because the codec
                // decoded the corresponding invoke.
                let value = match (self.codec, self.core.pending_f(token)) {
                    (Some(codec), Some(f)) => codec.encode_response(&f, &resp),
                    _ => Value::Nil,
                };
                self.core.close_op(token, OpKind::Ok, value);
            }
            Completion::Fail => self.core.close_op(token, OpKind::Fail, Value::Nil),
            Completion::Info => self.core.close_op(token, OpKind::Info, Value::Nil),
        }
    }
}
