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


def deployed(host: str) -> str:
    """The system the test's nodes give the host"""
    return str(settings["deployed"][host])


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


demos = {spec["name"]: spec for spec in settings["demos"]}


def snow_logs(machine: BaseMachine) -> set[str]:
    return set(machine.succeed("find /tmp -maxdepth 1 -name 'snow-*.log'").split())


class Demo:
    """One of the test's demos, recorded where it runs, under the prompt
    its user would see there"""

    def __init__(self, name: str, env: dict[str, str]):
        spec = demos[name]
        self.name = name
        self.machine = by_name[spec["runOn"]]
        self.out = f"/var/lib/demos/{name}"
        user = spec["user"]
        if user:
            home, flake = stage_for_user(self.machine, user)
        else:
            home, flake = "/root", FLAKE
        record = shlex.join([
            "record", "--prompt", f"alice@{spec['runOn']}:ops",
            "--name", name, "--title", spec["title"],
            self.out, spec["script"],
        ])
        variables = {
            "PATH": "/run/wrappers/bin:/run/current-system/sw/bin",
            "HOME": home,
            **env,
        }
        setenv = " ".join(
            shlex.quote(f"--setenv={name}={value}") for name, value in variables.items()
        )
        shell = f"{record} > /tmp/{name}.out 2>&1; echo $? > /tmp/{name}.status"
        self.logs_before = snow_logs(self.machine)
        self.started = time.monotonic()
        # In its own unit, so a demo that takes its machine down keeps what
        # it recorded
        run_as = f"--uid={user} " if user else ""
        self.machine.succeed(
            f"systemd-run --unit=demo-{name} {run_as}--working-directory={flake} {setenv} "
            f"/bin/sh -c {shlex.quote(shell)}"
        )

    def wait(self) -> "Demo":
        self.machine.wait_until_succeeds(f"test -e /tmp/{self.name}.status")
        self.seconds = time.monotonic() - self.started
        self.status = int(self.machine.succeed(f"cat /tmp/{self.name}.status"))
        self.raw = self.machine.succeed(f"cat /tmp/{self.name}.out")
        self.output = plain(self.raw)
        assert self.status != SCRIPT_FAILED, self.output
        assert cursor_shown(self.raw), ("snow left the cursor hidden", self.output)
        return self.collect()

    def collect(self) -> "Demo":
        self.machine.copy_from_machine(self.out, "demos")
        logs = sorted(snow_logs(self.machine) - self.logs_before)
        assert logs, "snow wrote no log"
        self.log = self.machine.succeed(f"cat {shlex.join(logs)}")
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


def demo(
    name: Optional[str] = None, wait: bool = True, env: Optional[dict[str, str]] = None
) -> Demo:
    """Runs the named demo, or the test's only one; with wait=False, returns
    at once, to be waited for"""
    assert demos, "this test has no demo"
    if name is None:
        assert len(demos) == 1, f"name one of the demos {sorted(demos)}"
        (name,) = demos
    started = Demo(name, env or {})
    return started.wait() if wait else started


def stage_flake(machine: BaseMachine) -> None:
    machine.succeed(f"cp -rL {settings['flake']} {FLAKE} && chmod -R u+w {FLAKE}")


def stage_for_user(machine: BaseMachine, user: str) -> tuple[str, str]:
    """A copy of the flake the user owns, and the demo's directory; returns
    the user's home and the flake's copy"""
    home = f"/home/{user}"
    flake = f"{home}/ops"
    machine.succeed(f"rm -rf {flake} && cp -rL {settings['flake']} {flake} && chmod -R u+w {flake}")
    machine.succeed(f"chown -R {user}: {flake} && install -d -o {user} /var/lib/demos")
    return home, flake


def setup() -> None:
    for host in hosts.values():
        host.start(allow_reboot=True)
    start_all()
    for name, host in hosts.items():
        host.wait_for_unit("install-system-profile.service")
        port = settings["sshPorts"][name] or 22
        deployer.wait_until_succeeds(f"ssh -o BatchMode=yes -p {port} root@{name} true")
        runs(name, base(name))
        boots(name, base(name))
    stage_flake(deployer)
    for runs_on in {spec["runOn"] for spec in demos.values()} - {settings["deployer"]}:
        stage_flake(by_name[runs_on])


setup()
