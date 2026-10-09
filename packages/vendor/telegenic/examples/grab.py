"""Full pipeline smoke test: connect, read features, stream frames.

python examples/grab.py <camera-ip> [n-frames]
"""

import sys
import time

import telegenic


def main() -> None:
    if len(sys.argv) < 2:
        sys.exit("usage: grab.py <camera-ip> [n-frames]")
    ip = sys.argv[1]
    n_frames = int(sys.argv[2]) if len(sys.argv) > 2 else 10

    cam = telegenic.Camera(ip)
    cam.connect()
    info = cam.device_info()
    print(f"{info.manufacturer} {info.model} (serial {info.serial})")

    for feature in ["Width", "Height", "PayloadSize"]:
        try:
            print(f"  {feature} = {cam.get_integer(feature)}")
        except telegenic.GenicamError as e:
            print(f"  {feature}: {e}")
    for feature in ["PixelFormat", "AcquisitionMode", "TriggerMode"]:
        try:
            print(f"  {feature} = {cam.get_enum(feature)}")
        except telegenic.GenicamError as e:
            print(f"  {feature}: {e}")

    with cam.start_acquisition() as acq:
        print(f"streaming to {acq.local_addr()} with packet size {acq.packet_size()}")
        received = 0
        deadline = time.monotonic() + 15.0
        while received < n_frames and time.monotonic() < deadline:
            frame = acq.wait_for(timeout=0.5)
            if frame is None:
                continue
            received += 1
            data = frame.data()
            mean = sum(data) / max(len(data), 1)
            print(
                f"frame {frame.frame_id:>4}  {frame.status}  "
                f"{frame.width}x{frame.height} {frame.pixel_format}  "
                f"{frame.received_size} bytes  ts {frame.timestamp_ns} ns  "
                f"mean {mean:.1f}"
            )
        stats = acq.stats()
        print(
            f"stream stats: {stats.completed_frames} complete, "
            f"{stats.failed_frames} failed, {stats.missing_packets} missing packets, "
            f"{stats.resend_requests} resends, {stats.underruns} underruns"
        )

    print(f"link stats: {cam.link_stats()}")
    cam.disconnect()
    assert received > 0, "no frames received"


if __name__ == "__main__":
    main()
