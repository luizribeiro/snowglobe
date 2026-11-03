"""Runs a command in a terminal of a fixed size and records it.

Usage: record CAST COLS ROWS COMMAND...

Writes an asciicast v2 file to CAST, which agg turns into a GIF, echoes the
command's output, and exits with its status.
"""

import codecs
import fcntl
import json
import os
import shlex
import struct
import sys
import termios
import time


def spawn(command, cols, rows):
    leader, follower = os.openpty()
    fcntl.ioctl(follower, termios.TIOCSWINSZ,
                struct.pack("HHHH", rows, cols, 0, 0))
    pid = os.fork()
    if pid == 0:
        os.close(leader)
        os.setsid()
        fcntl.ioctl(follower, termios.TIOCSCTTY, 0)
        for fd in range(3):
            os.dup2(follower, fd)
        os.close(follower)
        os.execvp(command[0], command)
    os.close(follower)
    return pid, leader


def main():
    cast, cols, rows, command = (
        sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4:])
    os.environ["TERM"] = "xterm-256color"
    started = time.monotonic()
    pid, leader = spawn(command, cols, rows)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")

    with open(cast, "w") as out:
        header = {"version": 2, "width": cols, "height": rows,
                  "timestamp": int(time.time()),
                  "env": {"TERM": "xterm-256color", "SHELL": "/bin/sh"}}
        out.write(json.dumps(header) + "\n")

        def record(text):
            event = [round(time.monotonic() - started, 6), "o", text]
            out.write(json.dumps(event) + "\n")
            sys.stdout.write(text)
            sys.stdout.flush()

        record(f"$ {shlex.join(command)}\r\n")
        while True:
            try:
                chunk = os.read(leader, 65536)
            except OSError:
                break
            if not chunk:
                break
            record(decoder.decode(chunk))

    _, status = os.waitpid(pid, 0)
    sys.exit(os.waitstatus_to_exitcode(status))


main()
