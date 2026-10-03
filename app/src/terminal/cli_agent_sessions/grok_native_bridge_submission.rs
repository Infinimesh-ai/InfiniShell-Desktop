//! 宿主编辑器授权只允许一次原子领取；关闭或替换会话后不能重新激活旧授权。

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use uuid::Uuid;
use warpui::EntityId;

use super::{CLIAgent, CLIAgentSessionsModel};

const AVAILABLE: u8 = 0;
const CLAIMED: u8 = 1;
const REVOKED: u8 = 2;

#[derive(Clone)]
pub(crate) struct SubmissionLease(Arc<AtomicU8>);

impl SubmissionLease {
    fn new() -> Self {
        Self(Arc::new(AtomicU8::new(AVAILABLE)))
    }

    /// 最后身份复核之后、第一字节写出之前是授权线性化点。
    pub(crate) fn claim_write(&self) -> bool {
        self.0
            .compare_exchange(AVAILABLE, CLAIMED, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
    }

    /// 只在同步 submit 已返回错误后检查；并发发送尚未结束时不能据此证明零字节。
    pub(crate) fn was_write_claimed(&self) -> bool {
        self.0.load(Ordering::SeqCst) == CLAIMED
    }

    fn revoke(&self) {
        let _ = self
            .0
            .compare_exchange(AVAILABLE, REVOKED, Ordering::SeqCst, Ordering::SeqCst);
    }
}

pub(super) struct SubmissionRegistration {
    generation: Uuid,
    session_id: String,
    lease: SubmissionLease,
}

impl Drop for SubmissionRegistration {
    fn drop(&mut self) {
        self.lease.revoke();
    }
}

impl CLIAgentSessionsModel {
    pub(crate) fn register_native_grok_submission(
        &mut self,
        view: EntityId,
        generation: Uuid,
        session_id: String,
    ) -> Option<SubmissionLease> {
        if !self.is_input_submission_current(view, generation)
            || !self.is_input_open(view)
            || self.native_grok_submissions.contains_key(&view)
            || self.sessions.get(&view).is_none_or(|session| {
                session.agent != CLIAgent::Grok
                    || session.remote_host.is_some()
                    || session
                        .session_context
                        .session_id
                        .as_ref()
                        .is_some_and(|observed| observed != &session_id)
            })
        {
            return None;
        }
        let lease = SubmissionLease::new();
        self.native_grok_submissions.insert(
            view,
            SubmissionRegistration {
                generation,
                session_id,
                lease: lease.clone(),
            },
        );
        Some(lease)
    }

    pub(super) fn revoke_native_grok_submission(&mut self, view: EntityId) {
        self.native_grok_submissions.remove(&view);
    }

    pub(super) fn revoke_stale_native_grok_submission(&mut self, view: EntityId) {
        let stale = self
            .native_grok_submissions
            .get(&view)
            .is_some_and(|registration| {
                !self.is_input_submission_current(view, registration.generation)
                    || self.sessions.get(&view).is_none_or(|session| {
                        session.agent != CLIAgent::Grok
                            || session.remote_host.is_some()
                            || session
                                .session_context
                                .session_id
                                .as_ref()
                                .is_some_and(|observed| observed != &registration.session_id)
                    })
            });
        if stale {
            self.revoke_native_grok_submission(view);
        }
    }
}

#[cfg(test)]
#[path = "grok_native_bridge_submission_tests.rs"]
mod tests;
