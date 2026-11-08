#!/usr/bin/env python3
"""Writes the scripted C16 capture for Rune's headless runner to stdout: pick an area,
start voice, then 60 s of continuous drawing with phrases and a still every 5 s."""
import json

SECONDS = 60
STROKE = 0.9
START = 1.0

def mark(t, name):
    return {"t": round(t, 3), "type": "mark", "name": name}

events = [
    mark(0.1, "key:capture"),
    {"t": 0.2, "type": "drag", "x": 100, "y": 60, "dx": 1000, "dy": 680, "dur": 0.1},
    mark(0.5, "key:voice"),
    mark(0.6, "voice:listening"),
    mark(0.8, "key:draw-down"),
]
for i in range(int(SECONDS / (STROKE + 0.1))):
    t = START + i * (STROKE + 0.1)
    y = 100 + (i * 37) % 600
    events.append({"t": round(t, 3), "type": "drag", "x": 150, "y": y, "dx": 900, "dy": 40 - (i % 80), "dur": STROKE})
for i in range(SECONDS // 2):
    t = START + i * 2 + 1.5
    events.append(mark(t, f"say:{t - 1.2:.2f}:{t - 0.2:.2f}:phrase number {i} of the scripted capture"))
for i in range(1, SECONDS // 5):
    events.append(mark(START + i * 5 + 0.95, "key:capture"))
events.append(mark(START + SECONDS, "key:draw-up"))
events.append(mark(START + SECONDS + 0.5, "end"))
events.sort(key=lambda e: e["t"])
print(json.dumps({
    "displays": [{"id": 1, "frame": [0, 0, 1280, 800], "scale": 2, "primary": True}],
    "pointer": [400, 300],
    "events": events,
}, indent=1))
