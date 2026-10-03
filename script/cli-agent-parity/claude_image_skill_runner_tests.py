"""图片与双技能在线收据的离线变异回归；合成数据不计原生验收。"""

import copy
import unittest

import run_claude_image_skill_live as runner


COMMANDS = runner.MULTI_COMMANDS
NATIVE = "00000000-0000-4000-8000-000000000100"


def proof():
    events = []
    traces = []
    for index in range(2):
        generation = f"00000000-0000-4000-8000-00000000010{index + 1}"
        turn = f"00000000-0000-4000-8000-00000000011{index + 1}"
        image_sha = str(index + 1) * 64
        array_sha = str(index + 3) * 64
        events.extend([
            {"event": "ready", "generation": generation, "selected_skill_commands": COMMANDS},
            {"event": "attachment_prepared", "generation": generation, "resumed": index == 1,
             "skill_commands": COMMANDS,
             "image_sha256": image_sha, "image_bytes": 100 + index, "media_type": "image/png",
             "native_array_sha256": array_sha},
        ])
        traces.extend([
            {"runtime_generation": generation, "type": "control_response",
             "image_skill_projection": {"tools": [], "selected_commands_registered": COMMANDS}},
            {"runtime_generation": generation, "type": "system",
             "image_skill_projection": {"tools": [], "native_model_matches_fixture": True}},
            {"runtime_generation": generation, "type": "user", "uuid": turn, "session_id": NATIVE,
             "rich_image_content_projection": {"array_sha256": array_sha, "array_bytes": 200,
                                               "images": [{"image_sha256": image_sha,
                                                           "image_bytes": 100 + index,
                                                           "media_type": "image/png"}]},
             "image_skill_projection": {"tools": []}},
        ])
        tools = []
        for position, command in enumerate(COMMANDS):
            approval_id = f"approval-{index}-{position}"
            control = f"control-{index}-{position}"
            tool_id = f"tool-{index}-{position}"
            input_sha = str(position + 5) * 64
            events.extend([
                {"event": "skill_approval", "generation": generation, "native_session_id": NATIVE,
                 "turn_id": turn, "approval_id": approval_id, "control_message_id": control,
                 "tool_use_id": tool_id, "input_sha256": input_sha,
                 "decision": "AllowOnce", "exact_registered_command": True},
                {"event": "skill_approval_resolved", "generation": generation,
                 "native_session_id": NATIVE, "turn_id": turn, "approval_id": approval_id,
                 "decision": "AllowOnce"},
                {"event": "skill_response_dispatched", "generation": generation,
                 "native_session_id": NATIVE, "turn_id": turn, "control_message_id": control},
            ])
            tools.append({"selected_skill_command": command, "tool_use_id": tool_id,
                          "input_sha256": input_sha})
        traces.append({"runtime_generation": generation, "type": "assistant", "session_id": NATIVE,
                       "image_skill_projection": {"tools": tools}})
        events.extend([
            {"event": "finished", "generation": generation, "native_session_id": NATIVE,
             "turn_id": turn, "outcome": "Completed", "result": f"MARKER_A\nMARKER_B\nCOLORS_{index}",
             "expected": f"MARKER_A\nMARKER_B\nCOLORS_{index}", "approved_commands": COMMANDS,
             "skill_approval_resolved": True, "skill_response_dispatched": True},
            {"event": "cleanup", "generation": generation, "native_session_id": NATIVE,
             "natural_exit": True,
             "cleanup_confirmed": True, "exit_code": 0},
        ])
    events.append({"event": "acceptance_passed", "native_session_id": NATIVE,
                   "skills_per_input": 2})
    return events, traces


class ImageMultiSkillReceiptTests(unittest.TestCase):
    def test_complete_two_generation_receipt_passes(self):
        events, traces = proof()
        self.assertTrue(runner.verify_native_receipts(events, traces, COMMANDS)[0])
        self.assertFalse(runner.verify_native_receipts(events, traces, [runner.COMMAND])[0])

    def test_second_approval_requires_first_resolution_and_dispatch(self):
        for kind in ("skill_approval_resolved", "skill_response_dispatched"):
            events, traces = proof()
            first = next(event for event in events if event["event"] == kind)
            events.remove(first)
            second = next(event for event in events if event["event"] == "skill_approval"
                          and event["approval_id"] == "approval-0-1")
            events.insert(events.index(second) + 1, first)
            with self.subTest(kind=kind):
                self.assertFalse(runner.verify_native_receipts(events, traces, COMMANDS)[0])

    def test_native_bytes_registration_tool_order_and_cleanup_are_required(self):
        changes = (
            lambda events, traces: traces[2]["rich_image_content_projection"]["images"][0].update(image_sha256="x" * 64),
            lambda events, traces: traces[0]["image_skill_projection"].update(selected_commands_registered=COMMANDS[:1]),
            lambda events, traces: traces[3]["image_skill_projection"]["tools"].reverse(),
            lambda events, traces: events[-2].update(exit_code=1),
        )
        for change in changes:
            events, traces = copy.deepcopy(proof())
            change(events, traces)
            with self.subTest(change=change):
                self.assertFalse(runner.verify_native_receipts(events, traces, COMMANDS)[0])


if __name__ == "__main__":
    unittest.main()
