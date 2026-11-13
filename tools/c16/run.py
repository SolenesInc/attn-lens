#!/usr/bin/env python3
"""Runs a command and writes its CPU seconds, wall seconds and peak RSS to a file.
  run.py OUT COMMAND..."""
import json, resource, subprocess, sys, time

begin = time.monotonic()
status = subprocess.call(sys.argv[2:])
used = resource.getrusage(resource.RUSAGE_CHILDREN)
with open(sys.argv[1], "w") as out:
    json.dump({"status": status, "cpu_s": round(used.ru_utime + used.ru_stime, 2),
               "wall_s": round(time.monotonic() - begin, 2), "peak_rss_mb": used.ru_maxrss // 1024}, out)
sys.exit(status)
