Download a supported on-device text model and add it to your practice Profile. Download time is outside this class's estimate.

1. Select it explicitly in FlowPilot's Profile model picker.
2. Send the one-word `cedar` prompt while connected, with no tools requested.
3. Disconnect the device from the network and send a new short prompt: `Reply with the single word birch. Do not use tools.`
4. Restore connectivity after the check.

**Expected:** the on-device inference can respond without contacting a hosted model. Record the outcome and any hardware or loading error. If a request tries to reach a provider, check the selected provider/model before downloading files again.

This checks the chosen inference path. Other nodes and tools in a workflow can still make network calls even when its model is local. Review those dependencies before claiming that an entire workflow stays on-device.

Local inference uses disk, memory, compute, and battery. Use the current card's requirements and an actual test on your target device to decide whether it is suitable.
