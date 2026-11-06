import json
import re
import shlex
import time
from pathlib import Path
from typing import Any, Optional

settings: dict[str, Any] = json.loads(Path(HARNESS).read_text())


def qemu(machine: BaseMachine) -> QemuMachine:
    """Reconnecting and starting with reboots allowed exist only on VMs"""
    assert isinstance(machine, QemuMachine), machine
    return machine


by_name = {machine.name: qemu(machine) for machine in machines}
hosts = {name: by_name[name] for name in settings["hosts"]}
deployer = by_name[settings["deployer"]]
FLAKE = "/root/ops"
# record's status when the demo script itself fails
SCRIPT_FAILED = 125
ansi = re.compile(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b[()][A-Z0-9]|\r")
cursor_toggle = re.compile(r"\x1b\[\?25([hl])")


def plain(text: str) -> str:
    return ansi.sub("", text)


def resolve(host: str, path: str) -> str:
    return hosts[host].succeed(f"readlink -f {path}").strip()


def running(host: str) -> str:
    return resolve(host, "/run/current-system")


def boots_into(host: str) -> str:
    return resolve(host, "/nix/var/nix/profiles/system")


def boot_id(host: str) -> str:
    return hosts[host].succeed("cat /proc/sys/kernel/random/boot_id").strip()


def reconnect(host: str) -> None:
    """After the host rebooted itself, which the driver doesn't notice"""
    hosts[host].connected = False
    hosts[host].wait_for_unit("multi-user.target")


def base(host: str) -> str:
    """The system the host booted at the start of the test"""
    return str(settings["base"][host])


def pushed(host: str) -> str:
    """The system the test's nodes give the host"""
    return str(settings["pushed"][host])


def runs(host: str, system: str) -> None:
    assert running(host) == system, (host, running(host), system)


def boots(host: str, system: str) -> None:
    assert boots_into(host) == system, (host, boots_into(host), system)


def cursor_shown(terminal_output: str) -> bool:
    """A terminal shows its cursor unless something hid it last"""
    return cursor_toggle.findall(terminal_output)[-1:] != ["l"]


def contains(text: str, expected: tuple[str, ...]) -> None:
    for part in expected:
        assert part in text, (part, text)


class Demo:
    """The test's demo, recorded where it runs, under the prompt the
    maintainer would see there"""

    def __init__(self, env: dict[str, str]):
        spec = settings["demo"]
        assert spec, "this test has no demo"
        self.machine = by_name[spec["runOn"]]
        record = shlex.join([
            "record", "--prompt", f"luiz@{spec['runOn']}:ops",
            "--name", spec["name"], "--title", spec["title"],
            "/var/lib/demo", spec["script"],
        ])
        variables = {
            "PATH": "/run/wrappers/bin:/run/current-system/sw/bin",
            "HOME": "/root",
            **env,
        }
        setenv = " ".join(
            shlex.quote(f"--setenv={name}={value}") for name, value in variables.items()
        )
        shell = f"{record} > /tmp/demo.out 2>&1; echo $? > /tmp/demo.status"
        self.started = time.monotonic()
        # In its own unit, so a demo that takes its machine down keeps what
        # it recorded
        self.machine.succeed(
            f"systemd-run --unit=demo --working-directory={FLAKE} {setenv} "
            f"/bin/sh -c {shlex.quote(shell)}"
        )

    def wait(self) -> "Demo":
        self.machine.wait_until_succeeds("test -e /tmp/demo.status")
        self.seconds = time.monotonic() - self.started
        self.status = int(self.machine.succeed("cat /tmp/demo.status"))
        self.raw = self.machine.succeed("cat /tmp/demo.out")
        self.output = plain(self.raw)
        assert self.status != SCRIPT_FAILED, self.output
        assert cursor_shown(self.raw), ("snow left the cursor hidden", self.output)
        return self.collect()

    def collect(self) -> "Demo":
        self.machine.copy_from_machine("/var/lib/demo")
        status, self.log = self.machine.execute("cat /tmp/snow-*.log")
        assert status == 0, "snow wrote no log"
        return self

    def has(self, *texts: str) -> None:
        contains(self.output, texts)

    def logged(self, *texts: str) -> None:
        contains(self.log, texts)

    def cursor_shown_at(self, *texts: str) -> None:
        self.cursor_at(True, texts)

    def cursor_hidden_at(self, *texts: str) -> None:
        self.cursor_at(False, texts)

    def cursor_at(self, shown: bool, texts: tuple[str, ...]) -> None:
        for text in texts:
            assert text in self.raw, (text, self.output)
            before = self.raw[: self.raw.index(text)]
            assert cursor_shown(before) == shown, f"cursor {'hidden' if shown else 'shown'} at {text!r}"

    def failed(self) -> None:
        assert self.status != 0, self.output

    def succeeded(self) -> None:
        assert self.status == 0, self.output


def demo(wait: bool = True, env: Optional[dict[str, str]] = None) -> Demo:
    """Runs the demo; with wait=False, returns at once, to be waited for"""
    started = Demo(env or {})
    return started.wait() if wait else started


def stage_flake(machine: BaseMachine) -> None:
    machine.succeed(f"cp -rL {settings['flake']} {FLAKE} && chmod -R u+w {FLAKE}")


def setup() -> None:
    for host in hosts.values():
        host.start(allow_reboot=True)
    start_all()
    for name, host in hosts.items():
        host.wait_for_unit("install-system-profile.service")
        port = settings["sshPorts"][name] or 22
        deployer.wait_until_succeeds(f"ssh -o BatchMode=yes -p {port} root@{name} true", timeout=60)
        runs(name, base(name))
        boots(name, base(name))
    stage_flake(deployer)
    if settings["demo"] and settings["demo"]["runOn"] != settings["deployer"]:
        stage_flake(by_name[settings["demo"]["runOn"]])


setup()
