"""Records a demo: a script types commands after a zsh prompt and answers
what they ask, in a terminal of a fixed size.

Usage: record --prompt USER@HOST:DIR --name NAME --title TITLE
              [--cols COLS] [--rows ROWS] OUT SCRIPT

SCRIPT is Python, run with these in scope:

  run(command)     waits for the command before it to exit, then types
                   COMMAND after the prompt and starts it
  expect(text, timeout=300)
                   waits for TEXT on screen, failing if the command exits
                   first or TEXT takes longer than TIMEOUT seconds
  press(*keys)     presses KEYS one at a time, slow enough to follow
  UP, DOWN, ENTER  keys to press

Writes OUT/demo.cast, an asciicast v2 file, and OUT/demo.json with NAME,
TITLE and the commands typed. Echoes what the commands show, and exits with
the last command's status, or 125, which commands mustn't use, if the script
fails. A script fails too if its last command runs longer than 10 minutes.

Takes the colors from the terminal look file at $TERMINAL_LOOK.
"""

import argparse
import json
import os
import re
import sys
import time
import traceback
from pathlib import Path

import pexpect

SCRIPT_FAILED = 125
EXPECT_TIMEOUT = 300
FINISH_TIMEOUT = 600
UP = "\x1b[A"
DOWN = "\x1b[B"
ENTER = "\r"
ESCAPES = r"(?:\x1b\[[0-9;?]*[A-Za-z])*"
with open(os.environ["TERMINAL_LOOK"]) as file:
    LOOK = json.load(file)
# Players that honor a recording's theme show it in the look's colors
THEME = {"fg": LOOK["fg"], "bg": LOOK["bg"],
         "palette": ":".join(LOOK["palette"])}
PROMPT = LOOK["prompt"]
USER = f"\x1b[{PROMPT['user']}m"
DIRECTORY = f"\x1b[{PROMPT['directory']}m"
COMMAND = f"\x1b[{PROMPT['command']}m"
RESET = "\x1b[0m"
TYPING_DELAY = 0.05
PAUSE_BEFORE_TYPING = 1.0
PAUSE_BEFORE_ANSWER = 1.0
PAUSE_BETWEEN_KEYS = 0.5


class ScriptFailed(Exception):
    pass


def zsh_prompt(spec):
    who, directory = spec.split(":", 1)
    return f"{USER}{who} {DIRECTORY}{directory} % {RESET}"


def on_screen(text):
    """TEXT, with any escape sequences the command draws between its
    characters"""
    return re.compile(ESCAPES.join(map(re.escape, text)))


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
        program, space, rest = command.partition(" ")
        self.write(COMMAND)
        self.type_out(program)
        self.write(RESET)
        self.type_out(space + rest)
        self.write("\r\n")


class Terminal:
    def __init__(self, cast, prompt, cols, rows, describe):
        self.cast = cast
        self.prompt = prompt
        self.dimensions = (rows, cols)
        self.describe = describe
        self.commands = []
        self.child = None
        self.status = None

    def run(self, command):
        self.finish()
        self.commands.append(command)
        self.describe(self.commands)
        self.cast.type_command(self.prompt, command)
        self.child = pexpect.spawn("/bin/sh", ["-c", command],
                                   dimensions=self.dimensions,
                                   encoding="utf-8", codec_errors="replace",
                                   maxread=65536)
        self.child.logfile_read = self.cast
        self.child.delaybeforesend = None

    def running(self):
        if self.child is None:
            raise ScriptFailed("nothing is running: call run() first")
        return self.child

    def expect(self, text, timeout=EXPECT_TIMEOUT):
        found = self.running().expect(
            [on_screen(text), pexpect.EOF, pexpect.TIMEOUT], timeout)
        if found == 1:
            raise ScriptFailed(f"the command exited without showing {text!r}")
        if found == 2:
            raise ScriptFailed(f"no {text!r} within {timeout}s")

    def pause(self, seconds):
        """Keeps recording meanwhile; False if the command exits first"""
        return self.running().expect(
            [pexpect.TIMEOUT, pexpect.EOF], seconds) == 0

    def press(self, *keys):
        delay = PAUSE_BEFORE_ANSWER
        for key in keys:
            if not self.pause(delay):
                raise ScriptFailed(f"the command exited before {key!r}")
            self.running().send(key)
            delay = PAUSE_BETWEEN_KEYS

    def finish(self):
        if self.child is None:
            return
        found = self.child.expect([pexpect.EOF, pexpect.TIMEOUT],
                                  FINISH_TIMEOUT)
        if found == 1:
            raise ScriptFailed(f"still running after {FINISH_TIMEOUT}s")
        self.child.wait()
        signal = self.child.signalstatus
        self.status = (128 + signal if signal is not None
                       else self.child.exitstatus)
        self.child = None

    def stop(self):
        if self.child is not None:
            self.child.close(force=True)

    def scope(self):
        return {"run": self.run, "expect": self.expect, "press": self.press,
                "UP": UP, "DOWN": DOWN, "ENTER": ENTER}


def parse():
    parser = argparse.ArgumentParser(prog="record")
    parser.add_argument("--prompt", required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--title", required=True)
    parser.add_argument("--cols", type=int, default=100)
    parser.add_argument("--rows", type=int, default=24)
    parser.add_argument("out", type=Path)
    parser.add_argument("script", type=Path)
    return parser.parse_args()


def play(terminal, script):
    exec(compile(script.read_text(), str(script), "exec"), terminal.scope())
    terminal.finish()
    if terminal.status is None:
        raise ScriptFailed("the script ran no command")
    return terminal.status


def main():
    args = parse()
    os.environ["TERM"] = "xterm-256color"
    prompt = zsh_prompt(args.prompt)
    args.out.mkdir(parents=True, exist_ok=True)

    # Rewritten before each command, so a command that reboots the machine
    # leaves it behind
    def describe(commands):
        about = {"name": args.name, "title": args.title,
                 "command": "; ".join(commands)}
        partial = args.out / "demo.json.partial"
        partial.write_text(json.dumps(about) + "\n")
        partial.replace(args.out / "demo.json")

    with open(args.out / "demo.cast", "w") as out:
        cast = Cast(out, args.cols, args.rows)
        terminal = Terminal(cast, prompt, args.cols, args.rows, describe)
        try:
            status = play(terminal, args.script)
        except BaseException:
            traceback.print_exc()
            sys.exit(SCRIPT_FAILED)
        finally:
            terminal.stop()
        cast.write(prompt)
    sys.exit(status)


if __name__ == "__main__":
    main()
