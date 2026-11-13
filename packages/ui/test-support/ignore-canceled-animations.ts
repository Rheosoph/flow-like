import Animation from "happy-dom/lib/animation/Animation.js";

// happy-dom rejects Animation.finished on cancel() without marking it handled; browsers mark it handled.
const cancel = Animation.prototype.cancel;
Animation.prototype.cancel = function cancelWithHandledFinished(
	this: Animation,
) {
	this.finished.catch(() => {});
	return cancel.call(this);
};
