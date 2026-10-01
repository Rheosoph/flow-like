import json
import os
from pathlib import Path
from flow_like import FlowLikeClient

payload = json.loads(Path(__file__).with_name("request.json").read_text())
with FlowLikeClient() as client:
    for event in client.trigger_event(os.environ["APP_ID"], os.environ["EVENT_ID"], payload):
        print(event.data)
