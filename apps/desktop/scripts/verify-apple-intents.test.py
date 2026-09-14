import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


spec = importlib.util.spec_from_file_location("verify_apple_intents", Path(__file__).with_name("verify-apple-intents.py"))
verifier = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verifier)


def valid_metadata():
    text_type = {"primitive": {"wrapper": {"typeIdentifier": 0}}}
    metadata = {
        "actions": {name: {
            "isDiscoverable": True, "mangledTypeName": "compiled-type",
            "fullyQualifiedTypeName": f"Flow_Like.{name}",
        } for name in verifier.SHORTCUTS},
        "autoShortcuts": [{"actionIdentifier": name} for name in verifier.SHORTCUTS],
        "entities": {"FlowEventResultEntity": {
            "transient": True,
            "properties": [{"identifier": name, "valueType": text_type} for name in ("text", "json")],
        }},
    }
    for name in ("AskFlowPilotIntent", "AskAppChatIntent"):
        metadata["actions"][name]["outputType"] = text_type
    metadata["actions"]["RunFlowEventIntent"]["outputType"] = {"entity": {"wrapper": {"typeName": "FlowEventResultEntity"}}}
    return metadata


class IntentMetadataTests(unittest.TestCase):
    def test_open_app_shortcut_must_be_extracted(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "extract.actionsdata"
            metadata = valid_metadata()
            metadata["autoShortcuts"] = [{"actionIdentifier": name} for name in verifier.SHORTCUTS if name != "OpenFlowAppIntent"]
            path.write_text(json.dumps(metadata))
            with self.assertRaisesRegex(ValueError, "Missing App Shortcuts: OpenFlowAppIntent"):
                verifier.verify_metadata(path, module="Flow_Like")

    def test_present_intents_with_empty_shortcuts_fail_release_check(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "extract.actionsdata"
            metadata = valid_metadata()
            metadata["autoShortcuts"] = []
            path.write_text(json.dumps(metadata))
            with self.assertRaisesRegex(ValueError, "Missing App Shortcuts"):
                verifier.verify_metadata(path, module="Flow_Like")
            verifier.verify_metadata(path, shortcuts=False, module="Flow_Like")
            metadata["autoShortcuts"] = [{"actionIdentifier": name} for name in verifier.SHORTCUTS]
            path.write_text(json.dumps(metadata))
            verifier.verify_metadata(path, module="Flow_Like")
            with self.assertRaisesRegex(ValueError, "not owned by target"):
                verifier.verify_metadata(path, module="UnexpectedModule")

    def test_dialog_only_action_cannot_pass_as_a_typed_shortcut(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "extract.actionsdata"
            metadata = valid_metadata()
            del metadata["actions"]["AskFlowPilotIntent"]["outputType"]
            path.write_text(json.dumps(metadata))
            with self.assertRaisesRegex(ValueError, "must return Text: AskFlowPilotIntent"):
                verifier.verify_metadata(path)

    def test_event_result_must_carry_both_properties_without_a_persistent_query(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "extract.actionsdata"
            metadata = valid_metadata()
            metadata["entities"]["FlowEventResultEntity"]["transient"] = False
            path.write_text(json.dumps(metadata))
            with self.assertRaisesRegex(ValueError, "must be transient"):
                verifier.verify_metadata(path)
            metadata["entities"]["FlowEventResultEntity"]["transient"] = True
            metadata["entities"]["FlowEventResultEntity"]["properties"].pop()
            path.write_text(json.dumps(metadata))
            with self.assertRaisesRegex(ValueError, "expose json as Text"):
                verifier.verify_metadata(path)


if __name__ == "__main__":
    unittest.main()
