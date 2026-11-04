"""Runs a command in a terminal of a fixed size and records it.

Usage: record [--prompt USER@HOST:DIR] [--answer TEXT KEYS]...
              CAST COLS ROWS -- COMMAND...

Writes an asciicast v2 file to CAST, echoes the command's output, and exits
with its status. The command is typed after a zsh prompt. Each --answer waits
for TEXT on screen, then presses KEYS, one at a time (written as Python
escapes: "\\x1b[B" is the down arrow).
"""

import argparse
import codecs
import json
import os
import re
import shlex
import sys
import time

import pexpect

ESCAPES = r"(?:\x1b\[[0-9;?]*[A-Za-z])*"
BOLD_GREEN, BOLD_BLUE, ARG0, RESET = (
    "\x1b[1;32m", "\x1b[1;34m", "\x1b[1;38;5;230m", "\x1b[0m")
# The maintainer's terminal colors ("Custom Monokai"), so players that honor
# a recording's theme show it as it looked in their terminal
THEME = {
    "fg": "#f8f7f2",
    "bg": "#1c1c1c",
    "palette": ":".join([
        "#262621", "#ca4238", "#bbd756", "#ded685",
        "#85c2d6", "#7b72c2", "#cf5a78", "#fefefe",
        "#717171", "#df3f67", "#bbd756", "#ded685",
        "#85c2d6", "#7b72c2", "#cf5a78", "#fefefe",
    ]),
}
TYPING_DELAY = 0.05
PAUSE_BEFORE_TYPING = 1.0
PAUSE_BEFORE_ANSWER = 1.0
PAUSE_BETWEEN_KEYS = 0.5


def zsh_prompt(spec):
    """The maintainer's zsh prompt: bold green user@host, bold blue dir"""
    who, directory = spec.split(":", 1)
    return f"{BOLD_GREEN}{who} {BOLD_BLUE}{directory} % {RESET}"


class Cast:
    def __init__(self, out, cols, rows):
        self.out = out
        self.started = time.monotonic()
        header = {"version": 2, "width": cols, "height": rows,
                  "timestamp": int(time.time()), "theme": THEME,
                  "env": {"TERM": "xterm-256color", "SHELL": "/bin/zsh"}}
        out.write(json.dumps(header) + "\n")

    def write(self, text):
        event = [round(time.monotonic() - self.started, 6), "o", text]
        self.out.write(json.dumps(event) + "\n")
        sys.stdout.write(text)
        self.flush()

    def flush(self):
        # A command that reboots the machine cuts the recording short
        self.out.flush()
        sys.stdout.flush()

    def type_out(self, text):
        for char in text:
            self.write(char)
            time.sleep(TYPING_DELAY)

    def type_command(self, prompt, command):
        self.write(prompt)
        time.sleep(PAUSE_BEFORE_TYPING)
        program, *rest = map(shlex.quote, command)
        self.write(ARG0)
        self.type_out(program)
        self.write(RESET)
        self.type_out("".join(" " + word for word in rest))
        self.write("\r\n")


def on_screen(text):
    """TEXT, with any escape sequences the command draws between its
    characters"""
    return re.compile(ESCAPES.join(map(re.escape, text)))


def pause(child, seconds):
    """Keeps recording meanwhile; False if the command exits first"""
    return child.expect([pexpect.TIMEOUT, pexpect.EOF], seconds) == 0


def answer(child, answers):
    """Presses keys once their text is on screen, slow enough to follow.
    Returns the texts that never appeared."""
    texts = [text for text, _ in answers]
    for index, (text, keys) in enumerate(answers):
        if child.expect([on_screen(text), pexpect.EOF], None) == 1:
            return texts[index:]
        delay = PAUSE_BEFORE_ANSWER
        for key in keys.split():
            if not pause(child, delay):
                return texts[index + 1:]
            child.send(codecs.decode(key, "unicode_escape"))
            delay = PAUSE_BETWEEN_KEYS
    return []


def parse():
    parser = argparse.ArgumentParser(prog="record")
    parser.add_argument("--prompt", default="luiz@sodium:ops")
    parser.add_argument("--answer", nargs=2, action="append", default=[],
                        metavar=("TEXT", "KEYS"))
    parser.add_argument("cast")
    parser.add_argument("cols", type=int)
    parser.add_argument("rows", type=int)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command[:1] == ["--"]:
        args.command = args.command[1:]
    return args


def run(cast, command, cols, rows, answers):
    child = pexpect.spawn(command[0], command[1:], dimensions=(rows, cols),
                          encoding="utf-8", codec_errors="replace",
                          maxread=65536)
    child.logfile_read = cast
    child.delaybeforesend = None
    unanswered = answer(child, answers)
    child.expect(pexpect.EOF, None)
    child.wait()
    if child.signalstatus is not None:
        return -child.signalstatus, unanswered
    return child.exitstatus, unanswered


def main():
    args = parse()
    os.environ["TERM"] = "xterm-256color"
    prompt = zsh_prompt(args.prompt)
    with open(args.cast, "w") as out:
        cast = Cast(out, args.cols, args.rows)
        cast.type_command(prompt, args.command)
        status, unanswered = run(cast, args.command, args.cols, args.rows,
                                 args.answer)
        cast.write(prompt)
    if unanswered:
        print(f"record: never saw {unanswered}", file=sys.stderr)
        sys.exit(status or 1)
    sys.exit(status)


main()
