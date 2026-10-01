A variable stores typed state for one invocation. Use it when multiple nodes need a value or the value changes while the run is executing.

1. In the variables panel create **Label**, type **String**, shape **Single**, default `draft`.
2. Drag Label onto the canvas twice: choose **Set** for one node and **Get** for the other.
3. Set the setter's value to `ready`. Wire execution from Simple Event through **Set Label** into Print Info.
4. Replace Print Info's Message connection with the value output of **Get Label**. Run.

**Expected:** Print Info writes `ready`. Bypass Set Label on the execution path and rerun: the message is `draft`, because each run starts with its own variable state. Restore the setter afterward.

Keep Label internal. **Exposed** lets a compatible caller supply a variable; **Runtime Configured** loads it from the runner's configuration. For a credential, combine **Secret** and **Runtime Configured**, entering the real value in Runtime Variables. This exercise needs no credential.

[Variables and their settings](https://docs.flow-like.com/studio/variables/)
