"""Create a deterministic, network-free 1280x720/30fps VP8 clip for optional A/B tests.

Requires PyAV and NumPy in the test environment; neither is an application dependency.
"""
from fractions import Fraction
from pathlib import Path
import av
import numpy as np

output = Path(__file__).resolve().parent.parent / 'artifacts/fixtures/720p30.webm'
output.parent.mkdir(parents=True, exist_ok=True)
with av.open(str(output), mode='w', format='webm') as container:
    stream = container.add_stream('libvpx', rate=30)
    stream.width, stream.height, stream.pix_fmt = 1280, 720, 'yuv420p'
    stream.thread_count = 2
    stream.options = {'deadline': 'realtime', 'cpu-used': '6', 'crf': '30'}
    for index in range(240):
        pixels = np.full((720, 1280, 3), (18, 35, 59), dtype=np.uint8)
        for band in range(16):
            x = (index * 7 + band * 80) % 1280
            color = ((band * 37) % 255, (band * 67 + 60) % 255, (band * 97 + 120) % 255)
            pixels[band * 45:band * 45 + 40, x:min(x + 120, 1280)] = color
        frame = av.VideoFrame.from_ndarray(pixels, format='rgb24')
        frame.pts, frame.time_base = index, Fraction(1, 30)
        for packet in stream.encode(frame):
            container.mux(packet)
    for packet in stream.encode():
        container.mux(packet)
with av.open(str(output)) as check:
    frames = sum(1 for _ in check.decode(video=0))
    if frames != 240 or abs(check.duration / av.time_base - 8) > 0.1:
        raise RuntimeError('Invalid fixture duration/frame count')
print(f'{output.name}: 1280x720, VP8, 30fps, 8s, {frames} frames, {output.stat().st_size} bytes')
