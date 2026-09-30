Upload `batch-a.csv` and `batch-b.csv` into `practice-drop/incoming` in app Storage. Keep outputs in a separate `practice-drop/output` folder.

1. Resolve the input folder and a manifest path with typed Path nodes under **Storage Dir**.
2. Add **Diff Directory** with that Root and Manifest. Select Checksum for the exercise.
3. Process both **Added** and **Updated** arrays. Use **For Each → Read to String → Write String**, copying each fixture to a deterministic corresponding output path.
4. After each successful output write, commit its selected path through **Write Directory Manifest** using the Diff session. Do not commit a failed path.
5. Run twice. Expect two outputs after the first run, and no unprocessed change after the second.
6. Replace batch-a.csv with `batch-a-updated.csv` under the original input name. Expect it in Updated, and one updated output at the same path.

For Deleted, this lab records the removed input name and retains the output for inspection. A production pipeline must choose retention or deletion deliberately; do not silently skip this case.

Test one failed output path in a scratch copy. The failed input should remain outstanding. A manifest records progress; it does not prevent two concurrent runs from seeing the same work. Deterministic replaceable outputs help here, but writes to another system need their own concurrency/repeat contract.
