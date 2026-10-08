"""离线攻击 V2 父子取消收据谓词；不启动 CLI，也不读取账户。"""

import copy
from pathlib import Path
import tempfile
import unittest

import run_claude_v2_write_cancel_live as runner


PARENT = "00000000-0000-4000-8000-000000000001"
CHILD = "00000000-0000-4000-8000-000000000002"
TURN = "00000000-0000-4000-8000-000000000003"
INTERRUPT = "00000000-0000-4000-8000-000000000004"
LATE = "00000000-0000-4000-8000-000000000005"
APPROVAL = "00000000-0000-4000-8000-000000000006"
PARENT_NATIVE = "00000000-0000-4000-8000-000000000007"
CHILD_NATIVE = "00000000-0000-4000-8000-000000000008"


def sample_events():
    def event(kind, **fields):
        return {"event": kind, **fields}

    def runtime(kind, fields):
        return event("runtime", task={"task_id": CHILD}, runtime={"kind": {kind: fields}})

    cleanup = {"native_process": "exited", "adapter_task_terminated": True,
               "event_journal_completed": True, "native_cleanup_sha256": "a" * 64}
    return [
        event("acceptance_started", scope=runner.SCOPE),
        event("parent_started", parent_task_id=PARENT, policy="ClaudeRestrictedFilesV2",
              target_absent=True, target_ref="cancelled-write.txt", content_sha256="b" * 64),
        event("parent_spawn_approved", approval_id="parent-tool", turn_id=PARENT, decision="AllowOnce"),
        event("parent_spawn_called", call_id="parent-call", turn_id=PARENT, tool="run_agents"),
        event("child_turn_started", task_id=CHILD, turn_id=TURN,
              native_session_id=CHILD_NATIVE),
        event("child_write_pending", task_id=CHILD, generation=1, approval_id=APPROVAL,
              turn_id=TURN, tool_use_id="toolu_fixed", exact_write_verified=True,
              target_absent=True, write_allowed=False),
        runtime("ApprovalRequested", {"approval_id": APPROVAL}),
        event("child_interrupt_submitted", task_id=CHILD, generation=1, turn_id=TURN,
              message_id=INTERRUPT, approval_id=APPROVAL, write_allowed=False),
        event("child_write_approval_cancelled", approval_id=APPROVAL, turn_id=TURN,
              native_session_id=CHILD_NATIVE, target_absent=True),
        runtime("ApprovalCancelled", {"approval_id": APPROVAL}),
        event("late_allow_submitted", approval_id=APPROVAL, message_id=LATE, decision="AllowOnce",
              same_cancelled_request_id=True),
        event("child_interrupt_ack", message_id=INTERRUPT, turn_id=TURN,
              native_session_id=CHILD_NATIVE),
        event("child_turn_cancelled", task_id=CHILD, turn_id=TURN,
              native_session_id=CHILD_NATIVE, outcome="Cancelled", target_absent=True),
        runtime("TurnFinished", {"turn_id": TURN, "outcome": "Cancelled"}),
        event("late_allow_rejected", approval_id=APPROVAL, message_id=LATE,
              reason_sha256="c" * 64, native_request_failed=True, target_absent=True),
        runtime("RequestFailed", {"message_id": LATE}),
        event("saved_cancel_chain_verified", parent_task_id=PARENT, child_task_id=CHILD,
              parent_generation=1, child_generation=1, parent_native_session_id=PARENT_NATIVE,
              child_native_session_id=CHILD_NATIVE, same_fixed_profile=True,
              parent_ceiling_saved=True, child_state="cancelled", late_allow_rejected=True,
              target_absent=True),
        event("cleanup_confirmed", task_id=PARENT, receipt=copy.deepcopy(cleanup)),
        event("cleanup_confirmed", task_id=CHILD, receipt=copy.deepcopy(cleanup)),
        event("acceptance_passed", scope=runner.SCOPE, waiting_write_cancel_verified=True,
              late_allow_rejected=True, target_file_unchanged=True,
              parent_child_ceiling_verified=True, all_runtime_hosts_cleaned=True),
    ]


class V2WriteCancelEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.events = sample_events()
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.target = Path(self.temporary.name) / "cancelled-write.txt"

    def audit(self):
        return runner.audit(self.events, 0,
                            "test result: ok. 1 passed; 0 failed; 0 ignored;", self.target)

    def event(self, kind):
        return runner.once(self.events, kind)

    def test_correlated_success_fixture(self):
        receipt, cleanup = self.audit()
        self.assertEqual(receipt["child_write_pending"]["approval_id"], APPROVAL)
        self.assertEqual(len(cleanup), 2)
        self.assertEqual(len(runner.project_public(self.events)), len(runner.EXPECTED_EVENTS))

    def test_late_allow_must_match_cancelled_request_and_native_rejection(self):
        self.event("late_allow_submitted")["approval_id"] = "different"
        with self.assertRaises(ValueError):
            self.audit()
        self.events = sample_events()
        self.events.remove(next(row for row in self.events if row["event"] == "runtime"
                                and "RequestFailed" in row["runtime"]["kind"]))
        with self.assertRaises(ValueError):
            self.audit()

    def test_extra_write_or_resolution_is_rejected(self):
        request = next(row for row in self.events if row["event"] == "runtime"
                       and "ApprovalRequested" in row["runtime"]["kind"])
        self.events.insert(self.events.index(request) + 1, copy.deepcopy(request))
        with self.assertRaises(ValueError):
            self.audit()
        self.events = sample_events()
        self.events.insert(9, {"event": "runtime", "task": {"task_id": CHILD},
                               "runtime": {"kind": {"ApprovalResolved": {"approval_id": APPROVAL}}}})
        with self.assertRaises(ValueError):
            self.audit()

    def test_file_effect_and_cleanup_are_independent(self):
        self.target.write_text("unexpected", encoding="utf-8")
        with self.assertRaises(ValueError):
            self.audit()
        self.target.unlink()
        next(row for row in self.events if row["event"] == "cleanup_confirmed"
             and row["task_id"] == CHILD)["receipt"]["native_process"] = "unconfirmed"
        with self.assertRaises(ValueError):
            self.audit()

    def test_native_cancel_and_order_cannot_be_omitted(self):
        self.events.remove(next(row for row in self.events if row["event"] == "runtime"
                                and "TurnFinished" in row["runtime"]["kind"]))
        with self.assertRaises(ValueError):
            self.audit()
        self.events = sample_events()
        first = self.events.index(self.event("child_write_approval_cancelled"))
        second = self.events.index(self.event("late_allow_submitted"))
        self.events[first], self.events[second] = self.events[second], self.events[first]
        with self.assertRaises(ValueError):
            self.audit()


if __name__ == "__main__":
    unittest.main()
