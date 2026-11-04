"""Runs a command in a terminal of a fixed size and records it.

Usage: record [--prompt USER@HOST:DIR] CAST COLS ROWS -- COMMAND...

Writes an asciicast v2 file to CAST, which agg turns into a GIF, echoes the
command's output, and exits with its status. The command is typed after a
zsh prompt.
"""

import argparse
import codecs
import fcntl
import json
import os
import re
import select
import shlex
import struct
import sys
import termios
import time

ANSI = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]")
BOLD_GREEN, BOLD_BLUE, ARG0, RESET = (
    "\x1b[1;32m", "\x1b[1;34m", "\x1b[1;38;5;230m", "\x1b[0m")
TYPING_DELAY = 0.05
PAUSE_BEFORE_TYPING = 1.0


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


def zsh_prompt(spec):
    """The maintainer's zsh prompt: bold green user@host, bold blue dir"""
    who, directory = spec.split(":", 1)
    return f"{BOLD_GREEN}{who} {BOLD_BLUE}{directory} % {RESET}"


class Cast:
    def __init__(self, out, cols, rows):
        self.out = out
        self.started = time.monotonic()
        header = {"version": 2, "width": cols, "height": rows,
                  "timestamp": int(time.time()),
                  "env": {"TERM": "xterm-256color", "SHELL": "/bin/zsh"}}
        out.write(json.dumps(header) + "\n")

    def record(self, text):
        event = [round(time.monotonic() - self.started, 6), "o", text]
        self.out.write(json.dumps(event) + "\n")
        # A command that reboots the machine cuts the recording short
        self.out.flush()
        sys.stdout.write(text)
        sys.stdout.flush()

    def type_out(self, text):
        for char in text:
            self.record(char)
            time.sleep(TYPING_DELAY)

    def type_command(self, prompt, command):
        self.record(prompt)
        time.sleep(PAUSE_BEFORE_TYPING)
        program, *rest = map(shlex.quote, command)
        self.record(ARG0)
        self.type_out(program)
        self.record(RESET)
        self.type_out("".join(" " + word for word in rest))
        self.record("\r\n")


def parse():
    parser = argparse.ArgumentParser(prog="record")
    parser.add_argument("--prompt", default="luiz@sodium:ops")
    parser.add_argument("cast")
    parser.add_argument("cols", type=int)
    parser.add_argument("rows", type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command[:1] == ["--"]:
        args.command = args.command[1:]
    return args


def run(cast, command, cols, rows):
    pid, leader = spawn(command, cols, rows)
    decoder = codecs.getincrementaldecoder("utf-8")("replace")
    while True:
        readable, _, _ = select.select([leader], [], [], 0.1)
        if readable:
            try:
                chunk = os.read(leader, 65536)
            except OSError:
                break
            if not chunk:
                break
            cast.record(decoder.decode(chunk))
    _, status = os.waitpid(pid, 0)
    return os.waitstatus_to_exitcode(status)


def main():
    args = parse()
    os.environ["TERM"] = "xterm-256color"
    prompt = zsh_prompt(args.prompt)
    with open(args.cast, "w") as out:
        cast = Cast(out, args.cols, args.rows)
        cast.type_command(prompt, args.command)
        status = run(cast, args.command, args.cols, args.rows)
        cast.record(prompt)
    sys.exit(status)


main()
