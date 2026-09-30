On Normalize Label, select Trim String and To Upper Case and choose **Collapse**. Name the resulting layer **Normalize text**.

**Check:** the outgoing value still reaches Print Info. Run ` Birch ` again and confirm `BIRCH`. Collapsing organizes the graph while preserving its behavior.

Open the layer. Its Start and Return nodes represent the boundary. If you add an input or change its type, inspect both the internal connections and every connection outside the layer. A label change alone does not supply a missing value.

A function adds a callable interface when several places need the same logic. Convert to a function when reuse is needed, after confirming unique boundary pin names and a supported execution entry. The [layers reference](https://docs.flow-like.com/studio/layers/) covers conversion.

A named layer is not proof of an implemented feature. An empty layer called **Human Review** does not wait for approval or prevent sending. Verify its internal behavior before depending on it. This course's normalizer contains only the two transformations you just tested.
